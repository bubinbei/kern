//! Win32 boundary. All HWND/GDI access is on the owning UI thread. Buffers are
//! NUL-terminated and live through synchronous API calls. RefCell::try_borrow_mut
//! rejects nested/reentrant callbacks rather than aliasing App. No raw App pointers.
use kern_protocol::*;
use std::{
    cell::{Cell, RefCell},
    io,
    path::PathBuf,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, WindowsAndMessaging::*},
};

const REFRESH: u16 = 101;
const START: u16 = 102;
const STOP: u16 = 103;
const SAVE: u16 = 104;
const CLOSE: u16 = 105;
const EXIT: u16 = 106;
const BG: u32 = 0x0021201d;
const PANEL: u32 = 0x00282828;
const TEXT: u32 = 0x00b2dbeb;
const AMBER: u32 = 0x002fbdfa;
const LIST: u16 = 120;
const INPUT: u16 = 121;
enum Work {
    Poll(std::time::Instant),
    Command(Request),
    Close,
}
struct Update {
    status: Option<Status>,
    plugins: Vec<PluginView>,
    message: Option<String>,
}
struct App {
    tx: SyncSender<Work>,
    rx: Receiver<Update>,
    status: HWND,
    list: HWND,
    edit: HWND,
    note: HWND,
    font: HFONT,
    brush: HBRUSH,
    panel: HBRUSH,
    plugins: Vec<PluginView>,
    revision: u64,
    scale: f32,
    controls: Vec<(HWND, i32, i32, i32, i32)>,
    last_status: String,
    last_list: Vec<String>,
    closing: bool,
}
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) };
static PALETTE: Cell<(usize,usize,usize)> = const { Cell::new((0,0,0)) }; }
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn request(endpoint: &Endpoint, req: Request) -> io::Result<Reply> {
    match call(endpoint, req)? {
        Reply::Error { code, message } => Err(io::Error::other(format!("{code}: {message}"))),
        v => Ok(v),
    }
}
fn worker(profile: PathBuf, session: String, rx: Receiver<Work>, tx: SyncSender<Update>) {
    while let Ok(work) = rx.recv() {
        if matches!(&work, Work::Poll(created) if created.elapsed() > std::time::Duration::from_secs(2))
        {
            continue;
        }
        let endpoint = match load_endpoint(&profile) {
            Ok(e) => e,
            Err(e) => {
                let _ = tx.try_send(Update {
                    status: None,
                    plugins: vec![],
                    message: Some(e.to_string()),
                });
                continue;
            }
        };
        if matches!(work, Work::Close) {
            let _ = request(
                &endpoint,
                Request::ShellClosed {
                    session: session.clone(),
                },
            );
            break;
        }
        let is_poll = matches!(work, Work::Poll(_));
        let mut message = None;
        if let Work::Command(command) = work {
            message = Some(match request(&endpoint, command) {
                Ok(_) => "Команда принята".into(),
                Err(e) => e.to_string(),
            });
        }
        let result = (|| -> io::Result<Update> {
            if is_poll {
                request(
                    &endpoint,
                    Request::ShellPulse {
                        session: session.clone(),
                    },
                )?;
            }
            let Reply::Status { status } = request(&endpoint, Request::Status)? else {
                return Err(io::Error::other("Invalid status reply"));
            };
            let Reply::Plugins {
                plugins,
                diagnostic,
            } = request(&endpoint, Request::Plugins)?
            else {
                return Err(io::Error::other("Invalid plugin reply"));
            };
            if !diagnostic.is_empty() {
                message = Some(diagnostic);
            }
            Ok(Update {
                status: Some(status),
                plugins,
                message,
            })
        })();
        let update = result.unwrap_or_else(|e| Update {
            status: None,
            plugins: vec![],
            message: Some(e.to_string()),
        });
        let _ = tx.try_send(update);
    }
    // Channel closure also handles a full queue during WM_CLOSE, without blocking UI.
    if let Ok(endpoint) = load_endpoint(&profile) {
        let _ = request(&endpoint, Request::ShellClosed { session });
    }
}

// SAFETY: caller owns window/control handles; these wrappers never store borrowed text pointers.
unsafe fn text(hwnd: HWND, value: &str) {
    unsafe {
        SetWindowTextW(hwnd, wide(value).as_ptr());
    }
}
unsafe fn control(
    app: &mut App,
    parent: HWND,
    class: &str,
    label: &str,
    id: u16,
    style: u32,
    rect: (i32, i32, i32, i32),
) -> HWND {
    unsafe {
        let (x, y, w, h) = rect;
        let s = app.scale;
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(label).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            (x as f32 * s) as i32,
            (y as f32 * s) as i32,
            (w as f32 * s) as i32,
            (h as f32 * s) as i32,
            parent,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(hwnd, WM_SETFONT, app.font as usize, 1);
        app.controls.push((hwnd, x, y, w, h));
        hwnd
    }
}
unsafe fn timer(app: &mut App) {
    unsafe {
        // UI message pump creates the heartbeat; a blocked UI cannot be hidden by a live worker.
        let _ = app.tx.try_send(Work::Poll(std::time::Instant::now()));
        while let Ok(update) = app.rx.try_recv() {
            if let Some(s) = update.status {
                app.revision = s.settings_revision;
                let label = format!(
                    "{}  ·  ядро {}  ·  запуск {}\r\nИсточники: {}   Пакеты: {}   Значения: {}   Журнал: {}\r\nОболочка: {}   Настройки: версия {}",
                    if s.disk_error.is_none() {
                        "● На связи"
                    } else {
                        "ОШИБКА ДИСКА"
                    },
                    s.version,
                    s.boot,
                    s.active_sources,
                    s.raw_batches,
                    s.raw_samples,
                    s.event_sequence,
                    if s.shell_connected {
                        "подключена"
                    } else {
                        "ожидание"
                    },
                    s.settings_revision
                );
                if app.last_status != label {
                    text(app.status, &label);
                    app.last_status = label;
                }
                // Keep an operator's in-progress edit; initialize only once.
                if GetWindowTextLengthW(app.edit) == 0 {
                    text(app.edit, &s.settings.stale_after_ms.to_string());
                }
            } else {
                text(
                    app.status,
                    "Нет связи с ядром. Регистрация проверяется отдельно.",
                );
                app.last_status.clear();
            }
            if let Some(note) = update.message {
                text(app.note, &note);
            }
            let rows: Vec<String> = update
                .plugins
                .iter()
                .map(|p| {
                    format!(
                        "{}  ·  {}  ·  {}{}",
                        p.name,
                        p.version,
                        match p.state.as_str() {
                            "running" => "работает",
                            "stopped" => "остановлен",
                            "closed" => "окно закрыто",
                            _ => "запуск",
                        },
                        if p.required {
                            "  [системный]"
                        } else {
                            ""
                        }
                    )
                })
                .collect();
            if rows != app.last_list {
                let selected = SendMessageW(app.list, LB_GETCURSEL, 0, 0);
                SendMessageW(app.list, WM_SETREDRAW, 0, 0);
                SendMessageW(app.list, LB_RESETCONTENT, 0, 0);
                for row in &rows {
                    SendMessageW(app.list, LB_ADDSTRING, 0, wide(row).as_ptr() as isize);
                }
                if !rows.is_empty() {
                    SendMessageW(app.list, LB_SETCURSEL, selected.max(0) as usize, 0);
                }
                SendMessageW(app.list, WM_SETREDRAW, 1, 0);
                InvalidateRect(app.list, null(), 1);
                app.last_list = rows;
            }
            app.plugins = update.plugins;
        }
    }
}
unsafe fn command(app: &mut App, hwnd: HWND, id: u16) {
    unsafe {
        match id {
            REFRESH => {
                let _ = app.tx.try_send(Work::Command(Request::PluginControl {
                    id: String::new(),
                    action: PluginAction::Rescan,
                }));
            }
            START | STOP => {
                let index = SendMessageW(app.list, LB_GETCURSEL, 0, 0);
                if let Some(plugin) = app.plugins.get(index as usize) {
                    if plugin.required {
                        text(
                            app.note,
                            "Системная оболочка обязательна. Окно можно закрыть, ядро продолжит работу.",
                        );
                        return;
                    }
                    let _ = app.tx.try_send(Work::Command(Request::PluginControl {
                        id: plugin.id.clone(),
                        action: if id == START {
                            PluginAction::Start
                        } else {
                            PluginAction::Stop
                        },
                    }));
                }
            }
            SAVE => {
                let mut value = [0u16; 32];
                GetWindowTextW(app.edit, value.as_mut_ptr(), 32);
                let n = value.iter().position(|c| *c == 0).unwrap_or(32);
                match String::from_utf16_lossy(&value[..n]).parse::<u64>() {
                    Ok(ms) if (100..=3_600_000).contains(&ms) => {
                        text(app.note, &format!("Сохранение: {ms} мс"));
                        let _ = app.tx.try_send(Work::Command(Request::SetSettings {
                            expected_revision: app.revision,
                            settings: Settings { stale_after_ms: ms },
                        }));
                    }
                    _ => text(app.note, "Введите целое число: от 100 до 3 600 000 мс."),
                }
            }
            CLOSE => {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            _ => {}
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: the OS supplies message-specific arguments; only documented message
    // layouts are dereferenced. App remains UI-thread-local and reentry is checked.
    unsafe {
        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return 0;
        }
        // Paint callbacks can reenter while control text changes. Their copied UI-thread
        // handles live until the window is destroyed; they never borrow mutable App.
        let paint = PALETTE.with(|p| {
            let (background, panel, font) = p.get();
            if background == 0 {
                return None;
            }
            match msg {
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                    SetTextColor(wp as HDC, TEXT);
                    SetBkColor(wp as HDC, if msg == WM_CTLCOLORSTATIC { BG } else { PANEL });
                    Some(if msg == WM_CTLCOLORSTATIC {
                        background as HBRUSH
                    } else {
                        panel as HBRUSH
                    } as isize)
                }
                WM_DRAWITEM => {
                    let d = &*(lp as *const DRAWITEMSTRUCT);
                    FillRect(d.hDC, &d.rcItem, panel as HBRUSH);
                    SetTextColor(d.hDC, AMBER);
                    SetBkMode(d.hDC, TRANSPARENT as i32);
                    let old_font = SelectObject(d.hDC, font as HFONT);
                    let mut label = [0u16; 128];
                    GetWindowTextW(d.hwndItem, label.as_mut_ptr(), 128);
                    let mut rect = d.rcItem;
                    DrawTextW(
                        d.hDC,
                        label.as_ptr(),
                        -1,
                        &mut rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                    if d.itemState & ODS_FOCUS != 0 {
                        DrawFocusRect(d.hDC, &d.rcItem);
                    }
                    SelectObject(d.hDC, old_font);
                    Some(1)
                }
                _ => None,
            }
        });
        if let Some(result) = paint {
            return result;
        }
        // Keep App unborrowed across the nested modal message pump so timers can
        // still prove UI progress while the operator considers shutdown.
        if msg == WM_COMMAND && (wp & 0xffff) as u16 == EXIT {
            if MessageBoxW(
                hwnd,
                wide("Остановить ядро и все запущенные им плагины?").as_ptr(),
                wide("KERN — остановка").as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            ) == IDYES
            {
                APP.with(|slot| {
                    if let Some(app) = slot.borrow().as_ref() {
                        let _ = app.tx.try_send(Work::Command(Request::Stop));
                    }
                });
            }
            return 0;
        }
        let handled = APP.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return None;
            };
            let app = slot.as_mut()?;
            match msg {
                WM_TIMER => {
                    timer(app);
                    Some(0)
                }
                WM_COMMAND => {
                    command(app, hwnd, (wp & 0xffff) as u16);
                    Some(0)
                }
                WM_CLOSE => {
                    if !app.closing {
                        app.closing = true;
                        let _ = app.tx.try_send(Work::Close);
                    }
                    DestroyWindow(hwnd);
                    Some(0)
                }
                _ => None,
            }
        });
        handled.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
    }
}

pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("KERN").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn run() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    validate_options(&args, &["--profile", "--session"])?;
    let argument = |name: &str| {
        args.windows(2)
            .find(|w| w[0] == name)
            .map(|w| w[1].clone())
            .ok_or_else(|| io::Error::other(format!("Missing {name}")))
    };
    let profile = PathBuf::from(argument("--profile")?);
    let session = argument("--session")?;
    let endpoint = load_endpoint(&profile)?;
    request(
        &endpoint,
        Request::ShellPulse {
            session: session.clone(),
        },
    )?;
    let (tx, work) = mpsc::sync_channel(4);
    let (updates, rx) = mpsc::sync_channel(4);
    let worker = thread::spawn(move || worker(profile, session, work, updates));
    // SAFETY: create/use/destroy all OS resources on this thread; borrowed vectors
    // survive their synchronous calls. No handles or pointers cross to the IPC worker.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        let instance = GetModuleHandleW(null());
        let class = wide("KernSystemShell");
        let brush = CreateSolidBrush(BG);
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hbrBackground: brush,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(io::Error::last_os_error());
        }
        let scale = GetDpiForSystem() as f32 / 96.;
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("KERN · Ядро и плагины").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (900.0 * scale) as i32,
            (650.0 * scale) as i32,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        let menu = CreateMenu();
        let program = CreatePopupMenu();
        AppendMenuW(
            program,
            MF_STRING,
            CLOSE as usize,
            wide("Закрыть окно").as_ptr(),
        );
        AppendMenuW(
            program,
            MF_STRING,
            EXIT as usize,
            wide("Остановить KERN…").as_ptr(),
        );
        AppendMenuW(menu, MF_POPUP, program as usize, wide("Программа").as_ptr());
        let plugins = CreatePopupMenu();
        AppendMenuW(
            plugins,
            MF_STRING,
            REFRESH as usize,
            wide("Обновить список").as_ptr(),
        );
        AppendMenuW(
            plugins,
            MF_STRING,
            START as usize,
            wide("Запустить выбранный").as_ptr(),
        );
        AppendMenuW(
            plugins,
            MF_STRING,
            STOP as usize,
            wide("Остановить выбранный").as_ptr(),
        );
        AppendMenuW(menu, MF_POPUP, plugins as usize, wide("Плагины").as_ptr());
        SetMenu(hwnd, menu);
        let font = CreateFontW(
            (-18.0 * scale) as i32,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Segoe UI").as_ptr(),
        );
        let mut app = App {
            tx,
            rx,
            status: null_mut(),
            list: null_mut(),
            edit: null_mut(),
            note: null_mut(),
            font,
            brush,
            panel: CreateSolidBrush(PANEL),
            plugins: vec![],
            revision: 0,
            scale,
            controls: vec![],
            last_status: String::new(),
            last_list: vec![],
            closing: false,
        };
        control(
            &mut app,
            hwnd,
            "STATIC",
            "KERN  /  Системная оболочка",
            0,
            0,
            (24, 18, 820, 30),
        );
        app.status = control(
            &mut app,
            hwnd,
            "STATIC",
            "Подключение к ядру…",
            0,
            0,
            (24, 60, 820, 90),
        );
        control(
            &mut app,
            hwnd,
            "STATIC",
            "Плагины",
            0,
            0,
            (24, 160, 820, 26),
        );
        app.list = control(
            &mut app,
            hwnd,
            "LISTBOX",
            "",
            LIST,
            WS_BORDER | WS_VSCROLL | LBS_NOTIFY as u32 | WS_TABSTOP,
            (24, 194, 820, 160),
        );
        for (label, id, x, w) in [
            ("Обновить список", REFRESH, 24, 190),
            ("Запустить", START, 226, 150),
            ("Остановить", STOP, 388, 150),
        ] {
            control(
                &mut app,
                hwnd,
                "BUTTON",
                label,
                id,
                BS_OWNERDRAW as u32 | WS_TABSTOP,
                (x, 366, w, 36),
            );
        }
        control(
            &mut app,
            hwnd,
            "STATIC",
            "Данные устаревают через, мс",
            0,
            0,
            (24, 424, 300, 28),
        );
        app.edit = control(
            &mut app,
            hwnd,
            "EDIT",
            "",
            INPUT,
            WS_BORDER | WS_TABSTOP | ES_NUMBER as u32,
            (330, 420, 150, 32),
        );
        SendMessageW(app.edit, EM_SETLIMITTEXT, 10, 0);
        control(
            &mut app,
            hwnd,
            "BUTTON",
            "Сохранить",
            SAVE,
            BS_OWNERDRAW as u32 | WS_TABSTOP,
            (494, 420, 150, 34),
        );
        app.note = control(
            &mut app,
            hwnd,
            "STATIC",
            "Папка plugins: один плагин — одна папка с plugin.json. Запуск — по кнопке.",
            0,
            0,
            (24, 470, 820, 62),
        );
        control(
            &mut app,
            hwnd,
            "BUTTON",
            "Закрыть окно",
            CLOSE,
            BS_OWNERDRAW as u32 | WS_TABSTOP,
            (24, 552, 190, 36),
        );
        control(
            &mut app,
            hwnd,
            "BUTTON",
            "Остановить KERN",
            EXIT,
            BS_OWNERDRAW as u32 | WS_TABSTOP,
            (638, 552, 206, 36),
        );
        if app.controls.iter().any(|c| c.0.is_null()) {
            DestroyWindow(hwnd);
            return Err(io::Error::last_os_error());
        }
        PALETTE.with(|p| p.set((app.brush as usize, app.panel as usize, app.font as usize)));
        APP.with(|slot| *slot.borrow_mut() = Some(app));
        SetTimer(hwnd, 1, 1000, None);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        APP.with(|slot| {
            if let Some(a) = slot.borrow_mut().as_mut() {
                timer(a);
            }
        });
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        KillTimer(hwnd, 1);
        APP.with(|slot| {
            if let Some(a) = slot.borrow_mut().take() {
                let _ = a.tx.try_send(Work::Close);
                DeleteObject(a.font);
                DeleteObject(a.panel);
            }
        });
        PALETTE.with(|p| p.set((0, 0, 0)));
        UnregisterClassW(class.as_ptr(), instance);
        DeleteObject(brush);
    }
    // Worker waits are bounded by local IPC timeouts. It never blocks the window thread.
    let _ = worker.join();
    Ok(())
}
