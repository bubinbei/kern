---
project: core
type: state
status: active
updated: 2026-10-24
---

# KERN — сквозной прототип 0.1.0

Rust1.96/Windows11. IM2480→redb→processor→параметры→uPlot/CSV/backup. Ядро — единственный писатель состояния; публикация после disk ack. redb Immediate/CRC,cache16МиБ,порог4ГиБ. KJN сохранён, backend не смешивать; R2-импорт впереди.

Источник im2480: эксклюзивный COM,FC03,0009=0,6типизированных каналов/модуль,слоты0–15; резерв100×10 ядра. Сохраняется не полный кадр. Missing по модулю,bounded повтор той же batch. Пока один управляемый IM-процесс/профиль. Команды через PID-mailbox≤32КиБ,статус≤128КиБ; другой COM-владелец не создаётся.

«Модули»: входы слева/выходы справа,drag/два нажатия,размыкание/ветвление,поиск1–100,настройки/статистика. Провода меняют карту,весь настроенный raw сохраняется. Hot config: проверка expected+диска,новая сессия,возврат при занятом порте; номера slot*6 постоянны. История64 before/after локальная,в JSONL backup ещё не входит. Карта и линия — отдельные операции. ДОЛ только исходный A/B; иных режимов/записи в прибор нет.

Карты linear-v1: immutable revision,raw с активной картой,params с raw/revision/algorithm; enabled=false сохраняет точки,текущий snapshot null/disabled. Maps≤256,params≤32. Durable position + calibration_at: restart без полного чтения прошлого. Stateful/осреднений нет. float_roundtrip обязателен; свежесть от входа.

kern-shell native обязателен. charts — отдельный loopback Rust-host/ключ,локальные JS/CSS/uPlot,окно Edge/Chrome. Закрытие UI не останавливает запись. Modules встроен в host с проверкой пакета,универсальный web SDK впереди. LAN/роли/интернет — NETWORK_PLAN.

CSV/backup отдельны,CRC/позиции; restore в новый профиль. Резерв может отставать,общий drain/периоды ещё впереди. Надзор processor/csv/backup:3старта/60с→safe;lease15с/нет прогресса20с,только свои Child;stop запрещает restart.

Контракт/приёмка: docs/IM2480.md,PROTOTYPE.md,CORE_VALIDATION.md. dist/Kern-Prototype-0.1.0/Start-Kern.cmd; карты ТЕСТОВЫЕ. 38Rust/13Python;17COM/9management/13UI,18core/6pipeline/20editor/9charts. Стенд обновлён с checkpoint/drain,карты/конфиг/история сохранены;5модулей/30свежих каналов.

Не приняты: физика/шкалаАЦП,Win10/слабыеПК/macOS/GPU/24–72ч/power-loss. Далее: общий журнал config/replay/drain,отказы,периоды,экземпляры источников. Пути ASCII; публичная граница — AGENTS/docs/MATERIALS.

План каталога/наборов/расчётов: docs/PARAMETER_PRESETS_PLAN.md; ещё не реализация.
