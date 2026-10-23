---
project: core
type: state
status: active
updated: 2026-10-23
---

# KERN — raw-ядро и нативная оболочка 0.1.0

Рабочий независимый C0–C1/raw-срез + выбранные C2/C6: protocol/engine/core/demo/plugins/shell, Rust1.96. Ядро отдельно, без GUI/приборов. Один писатель состояния; видимость после disk ack; KJNv1 CRC/sequence/sync,4МиБ×4096, потоковый reader, новый сегмент на boot, сохранённый неполный хвост/recovery-копия. Резерв100×10, ID/слоты/сессии/типы/качество/повторы; свежесть от монотонного приёма, не sync. Карты/интерпретатор/физические параметры/подписки/рабочие писатели/backup ещё не реализованы.

Desktop: `kern-core run` запускает отдельный обязательный kern-shell.exe; serve остаётся headless. Нативные Win32/GDI меню/статусы/настройка свежести/список/ручной start-stop. Крестик закрывает UI, не ядро; `shell --profile` открывает снова. UI heartbeat от message pump,10с timeout,1с delay,3 запуска/60с→safe mode до явного открытия. Только свои Child handles. Дополнительные пакеты plugins/<id>/plugin.json+exe,32entries/16КиБ,без автозапуска/сохранённого desired state. stop stdin/2с→kill, demo кооперативно закрывает сессию; forced kill может оставить stale/занятый источник. Capabilities/подписи/песочница/общий надзор плагинов впереди. Unsafe только Win32 boundary оболочки; core/protocol/loader/demo forbid.

Windows11/22631:20Rust+13Python,18raw+14desktop сценариев,fmt/clippy/release; forced UI crash/приостановка UI-потока/restart budget/закрытие не прерывают raw, независимый CRC reader. Короткий idle UI≈10,8МиБ+core≈4,7МиБ;CPU0поразрешениюсчётчика,не бюджет. Win10/слабыйIntel/24–72ч/GPU/power-loss/полныйC2 не приняты. Исходники/отчёты: CORE_VALIDATION,PLUGINS.

Веб среда для оператора без программирования — отдельный будущий W1: декларативные виджеты/шаблоны→ограниченный API/права→прототип/сравнение на слабомПК. Ответственные участки Rust; web engine не выбран. Подробное руководство ядра ведётся; помощь меняющегосяUI отложена. dist/Kern-Core-0.1.0, Start-Kern.cmd. Не трогать старые профили/приборы. KERN/трислоя/Ember-Gruvbox; разговорные ориентиры не расшифровывать в репозитории.

Далее: продолжить C2 и стандартный интерпретатор C3 с версиями/активацией/replay. Win10x64 первая цель; Mac2012/macOS позже.
