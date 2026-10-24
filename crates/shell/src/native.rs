//! Win32 boundary. All HWND/GDI access is on the owning UI thread. Buffers are
//! NUL-terminated and live through synchronous API calls. RefCell::try_borrow_mut
//! rejects nested/reentrant callbacks rather than aliasing App. No raw App pointers.
use crate::health::{DiskUsage, Monitor, Tone, bytes};
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
    Storage::FileSystem::GetDiskFreeSpaceExW,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, Shell::*, WindowsAndMessaging::*},
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
    sampled_at: std::time::Instant,
    status: Option<Status>,
    disk: DiskUsage,
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
    monitor: Monitor,
    disk: DiskUsage,
    tray: Tray,
    epoch: std::time::Instant,
}
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) };
static PALETTE: Cell<(usize,usize,usize)> = const { Cell::new((0,0,0)) }; }
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
const TRAY_CALLBACK: u32 = WM_APP + 12;
const TRAY_ID: u32 = 1;
const TRAY_KEYSELECT: u32 = NIN_SELECT | 1;
const TRAY_SHOW: u32 = 201;
const TRAY_HIDE: u32 = 202;
struct Tray {
    hwnd: HWND,
    icons: Vec<HICON>,
    taskbar_created: u32,
    present: bool,
    frame: usize,
    tip: String,
    attempt_at: std::time::Instant,
}
impl Tray {
    unsafe fn new(hwnd: HWND) -> io::Result<Self> {
        unsafe {
            let mut icons = Vec::new();
            for rgb in [
                [46u8, 207, 137],
                [250, 189, 47],
                [251, 73, 52],
                [92, 172, 220],
                [146, 131, 116],
            ] {
                for bright in [false, true] {
                    let mut mask = [255u8; 64];
                    let mut pixels = [0u8; 1024];
                    for y in 1..15usize {
                        for x in 1..15usize {
                            if (x == 1 || x == 14) && (y == 1 || y == 14) {
                                continue;
                            }
                            let dy = (x as f32 - 7.5).abs();
                            let glyph = (3..13).contains(&x)
                                && (((y as f32 - 5.).abs() + dy * 0.5 < 2.)
                                    || ((y as f32 - (8. + dy * 0.5)).abs() < 0.8)
                                    || ((y as f32 - (11. + dy * 0.4)).abs() < 0.7));
                            let i = ((15 - y) * 16 + x) * 4;
                            mask[(15 - y) * 4 + x / 8] &= !(0x80 >> (x % 8));
                            let c = if glyph {
                                [29, 32, 33]
                            } else if bright {
                                rgb
                            } else {
                                rgb.map(|v| (u16::from(v) * 3 / 5) as u8)
                            };
                            pixels[i..i + 4].copy_from_slice(&[c[2], c[1], c[0], 255]);
                        }
                    }
                    let icon = CreateIcon(
                        GetModuleHandleW(null()),
                        16,
                        16,
                        1,
                        32,
                        mask.as_ptr(),
                        pixels.as_ptr(),
                    );
                    if icon.is_null() {
                        for i in icons {
                            DestroyIcon(i);
                        }
                        return Err(io::Error::last_os_error());
                    }
                    icons.push(icon);
                }
            }
            let mut tray = Self {
                hwnd,
                icons,
                taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
                present: false,
                frame: Tone::Gray.index() * 2,
                tip: "KERN · подключение к ядру".into(),
                attempt_at: std::time::Instant::now(),
            };
            tray.recreate();
            Ok(tray)
        }
    }
    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: TRAY_CALLBACK,
            hIcon: self.icons[self.frame],
            ..Default::default()
        };
        for (out, c) in data.szTip[..127].iter_mut().zip(self.tip.encode_utf16()) {
            *out = c;
        }
        data
    }
    unsafe fn recreate(&mut self) {
        unsafe {
            let mut data = self.data();
            Shell_NotifyIconW(NIM_DELETE, &data);
            self.present = Shell_NotifyIconW(NIM_ADD, &data) != 0;
            SendMessageW(
                self.hwnd,
                WM_SETICON,
                ICON_SMALL as usize,
                self.icons[self.frame] as isize,
            );
            self.attempt_at = std::time::Instant::now();
            if self.present {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
    }
    unsafe fn update(
        &mut self,
        monitor: &Monitor,
        plugins: &[PluginView],
        disk: &DiskUsage,
        now: u64,
    ) {
        unsafe {
            let (tone, _, pulse) = monitor.state(plugins, disk, now);
            let frame = tone.index() * 2 + usize::from(pulse);
            let tip = monitor.tip(plugins, disk, now);
            if frame != self.frame || tip != self.tip {
                self.frame = frame;
                SendMessageW(
                    self.hwnd,
                    WM_SETICON,
                    ICON_SMALL as usize,
                    self.icons[self.frame] as isize,
                );
                self.tip = tip;
                if self.present {
                    self.present = Shell_NotifyIconW(NIM_MODIFY, &self.data()) != 0;
                }
            }
            if !self.present && self.attempt_at.elapsed() >= std::time::Duration::from_secs(5) {
                self.recreate();
            }
        }
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        // SAFETY: owned icon/window handles stay on their creating UI thread.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &self.data());
            for icon in &self.icons {
                DestroyIcon(*icon);
            }
        }
    }
}
fn tray_lines(app: &App) -> Vec<String> {
    let (_, label, _) = app.monitor.state(
        &app.plugins,
        &app.disk,
        app.epoch.elapsed().as_millis() as u64,
    );
    let mut lines = vec![
        format!("KERN · {label}"),
        app.disk.summary(),
        format!(
            "База: {} · CSV: {} · Резерв: {}",
            bytes(app.disk.primary),
            bytes(app.disk.csv),
            bytes(app.disk.backup)
        ),
    ];
    for id in ["csv", "backup"] {
        let state =
            app.plugins
                .iter()
                .find(|p| p.id == id)
                .map_or("не подключён", |p| {
                    if p.state == "running" {
                        "работает"
                    } else {
                        "остановлен"
                    }
                });
        lines.push(format!(
            "{}: {state}",
            if id == "csv" { "CSV" } else { "Резерв" }
        ));
    }
    lines
}
unsafe fn tray_menu(hwnd: HWND, lines: &[String]) {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        for line in lines {
            AppendMenuW(menu, MF_STRING | MF_DISABLED, 0, wide(line).as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(
            menu,
            MF_STRING,
            TRAY_SHOW as usize,
            wide("Состояние ядра").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            TRAY_HIDE as usize,
            wide("Свернуть в трей").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            EXIT as usize,
            wide("Остановить KERN…").as_ptr(),
        );
        let mut pos = POINT::default();
        GetCursorPos(&mut pos);
        SetForegroundWindow(hwnd);
        let selected = TrackPopupMenuEx(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            pos.x,
            pos.y,
            hwnd,
            null(),
        );
        DestroyMenu(menu);
        match selected as u32 {
            TRAY_SHOW => {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            TRAY_HIDE => {
                ShowWindow(hwnd, SW_HIDE);
            }
            n if n == u32::from(EXIT) => {
                PostMessageW(hwnd, WM_COMMAND, EXIT as usize, 0);
            }
            _ => {}
        }
        PostMessageW(hwnd, WM_NULL, 0, 0);
    }
}
fn request(endpoint: &Endpoint, req: Request) -> io::Result<Reply> {
    match call(endpoint, req)? {
        Reply::Error { code, message } => Err(io::Error::other(format!("{code}: {message}"))),
        v => Ok(v),
    }
}
fn meter(profile: PathBuf, tx: SyncSender<(std::time::Instant, DiskUsage)>) {
    loop {
        let mut free = 0u64;
        // SAFETY: live u64 outputs and a UTF-16 buffer owned for the synchronous OS call.
        let available = unsafe {
            GetDiskFreeSpaceExW(
                wide(&format!("{}\\", profile.display())).as_ptr(),
                &mut free,
                null_mut(),
                null_mut(),
            )
        };
        let disk = DiskUsage::read(&profile, if available != 0 { Some(free) } else { None });
        if let Err(mpsc::TrySendError::Disconnected(_)) =
            tx.try_send((std::time::Instant::now(), disk))
        {
            break;
        }
        thread::sleep(std::time::Duration::from_secs(5));
    }
}
fn worker(
    profile: PathBuf,
    session: String,
    rx: Receiver<Work>,
    tx: SyncSender<Update>,
    disk_rx: Receiver<(std::time::Instant, DiskUsage)>,
) {
    let mut disk = DiskUsage::default();
    let mut disk_at = None::<std::time::Instant>;
    while let Ok(work) = rx.recv() {
        if matches!(&work,Work::Poll(created) if created.elapsed()>std::time::Duration::from_secs(2))
        {
            continue;
        }
        for (at, next) in disk_rx.try_iter() {
            disk = next;
            disk_at = Some(at);
        }
        let visible_disk =
            if disk_at.is_some_and(|t| t.elapsed() <= std::time::Duration::from_secs(15)) {
                disk.clone()
            } else {
                DiskUsage::default()
            };
        let endpoint = match load_endpoint(&profile) {
            Ok(e) => e,
            Err(e) => {
                let _ = tx.try_send(Update {
                    sampled_at: std::time::Instant::now(),
                    status: None,
                    disk: visible_disk.clone(),
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
            let sampled_at = std::time::Instant::now();
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
                sampled_at,
                status: Some(status),
                disk: visible_disk.clone(),
                plugins,
                message,
            })
        })();
        let update = result.unwrap_or_else(|e| Update {
            sampled_at: std::time::Instant::now(),
            status: None,
            disk: visible_disk.clone(),
            plugins: vec![],
            message: Some(e.to_string()),
        });
        let _ = tx.try_send(update);
    }
    // Channel closure also handles a full queue during actual shell exit, without blocking UI.
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
            app.disk = update.disk;
            let sampled_ms = (app.epoch.elapsed().as_millis() as u64)
                .saturating_sub(update.sampled_at.elapsed().as_millis() as u64);
            app.monitor.observe(update.status.clone(), sampled_ms);
            if let Some(s) = update.status {
                app.revision = s.settings_revision;
                let label = format!(
                    "{}  ·  ядро {}  ·  запуск {}\r\nИсточники: {}   Пакеты: {}   Значения: {}   Журнал: {}\r\nОболочка: {}   Настройки: версия {}\r\n{}",
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
                    s.settings_revision,
                    app.disk.summary()
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
        if msg == TRAY_CALLBACK {
            let event = (lp as u32) & 0xffff;
            if event == WM_CONTEXTMENU || event == WM_RBUTTONUP {
                let lines = APP.with(|slot| {
                    slot.try_borrow()
                        .ok()
                        .and_then(|app| app.as_ref().map(tray_lines))
                });
                let Some(lines) = lines else {
                    return 0;
                };
                tray_menu(hwnd, &lines);
                return 0;
            }
            if matches!(
                event,
                NIN_SELECT | TRAY_KEYSELECT | WM_LBUTTONUP | WM_LBUTTONDBLCLK
            ) {
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
                return 0;
            }
        }
        let handled = APP.with(|slot| {
            let Ok(mut slot) = slot.try_borrow_mut() else {
                return None;
            };
            let app = slot.as_mut()?;
            if app.tray.taskbar_created != 0 && msg == app.tray.taskbar_created {
                app.tray.recreate();
                return Some(0);
            }
            match msg {
                WM_TIMER => {
                    if wp == 1 {
                        timer(app);
                    }
                    app.tray.update(
                        &app.monitor,
                        &app.plugins,
                        &app.disk,
                        app.epoch.elapsed().as_millis() as u64,
                    );
                    Some(0)
                }
                WM_COMMAND => {
                    command(app, hwnd, (wp & 0xffff) as u16);
                    Some(0)
                }
                WM_CLOSE => {
                    ShowWindow(hwnd, SW_HIDE);
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
    let (disk_tx, disk_rx) = mpsc::sync_channel(1);
    let disk_profile = profile.clone();
    // A stalled disk probe must not block the shell heartbeat. The bounded meter is
    // intentionally not joined at UI exit: process exit releases any blocked OS call.
    thread::Builder::new()
        .name("kern-storage-meter".into())
        .stack_size(256 * 1024)
        .spawn(move || meter(disk_profile, disk_tx))?;
    let worker = thread::spawn(move || worker(profile, session, work, updates, disk_rx));
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
            wide("Свернуть в трей").as_ptr(),
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
            monitor: Monitor::default(),
            disk: DiskUsage::default(),
            tray: Tray::new(hwnd)?,
            epoch: std::time::Instant::now(),
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
        SetTimer(hwnd, 2, 250, None);
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
        KillTimer(hwnd, 2);
        APP.with(|slot| {
            if let Some(a) = slot.borrow_mut().as_mut() {
                Shell_NotifyIconW(NIM_DELETE, &a.tray.data());
                a.tray.present = false;
            }
        });
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
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
