---
project: core
type: state
status: active
updated: 2026-10-23
---

# KERN — raw, shell, im2480 0.1.0

Независимый Rust1.96 C0–C1/raw + выбранные C2/C5/C6. Один писатель состояния, видимость после disk ack. KJNv1 CRC/sequence/sync,4МиБ×4096; потоковый reader, новый сегмент на boot, сохранённый хвост/recovery-копия. Резерв100×10; стабильные source ID/слоты/сессии/типы/качество/повторы, свежесть от монотонного приёма. Карты/интерпретатор/параметры/подписки/рабочие писатели/backup ещё не реализованы.

Desktop: `run`→отдельный обязательный kern-shell; serve headless. Меню/настройка свежести/каталог/ручной start-stop. UI close не останавливает raw; `shell --profile` открывает снова. UI heartbeat10с,delay1с,3запуска/60с→safe mode; модальный диалог не скрывает heartbeat. Только свои Child. Пакеты plugins/<id>/plugin.json+exe,32entries/16КиБ;без autostart/desired-state persistence. stop stdin/2с→kill; forced kill может оставить занятую сессию.

Плагин im2480: собственный процесс/эксклюзивный COM,FC03,8N1,до16модулей×6=96каналов; явные постоянные slots0..15. Проверка identity и0009=0,CRC/адреса/длины/конечности. А1–А3 unsigned,DOL signed32,2float; остальные регистры не архивируются целиком. Отказ адреса→только его missing, опрос остальных продолжается. Одна ожидающая batch,повтор того же payload/sequence при busy/uncertain с backoff. Конфиг config.json локальный/не Git; шаблон без порта, упаковка сохраняет настройки. Диагностика plugin-status/source.json неавторитетна. Порт unplug/новый boot/crash требуют явного restart/close; общий supervisor/capabilities/песочница впереди. Unsafe только OS UI и native COM boundary, safe decoder forbid.

Проверено Windows11:26Rust+13Python,17COMсценариев с независимым peer/CRC reader; ранее18raw+14desktop. Живой существующий эмулятор:5модулей/30каналов/10batch/60samples; не менялся. Комплект dist/Kern-Core-0.1.0,Start-Kern.cmd,im2480 готов по прежним локальным настройкам. Win10/слабыйПК/GPU/24–72ч/USB-RS485/power-loss/полныйC2 не приняты.

Утверждены redb+uPlot; код ещё KJN/native. STORAGE_CHARTS_PLAN:R1raw→R2импорт→R3карты/replay→R4API/uPlot→R5writers/backup→R6стенд. Далее NETWORK_PLAN:N0общий контракт сейчас; браузер/KERN-клиент с общими веб экранами, шлюз отдельно; LAN→интернет-просмотр→офисное управление с правами/аудитом/отзывом. Сеть не включать, далее R1синтетика. Host/W1позже, ответственные плагины Rust. Настройки im2480 отдельно; пути ASCII, старые профили/приборы не трогать; KERN/Ember, ориентиры не расшифровывать.
