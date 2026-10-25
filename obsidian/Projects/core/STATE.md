---
project: core
type: state
status: active
updated: 2026-10-25
---

# KERN — сквозной прототип 0.1.0

Rust1.96/Windows11. IM2480→redb→processor→параметры→uPlot/CSV/backup. Ядро единственный писатель, публикация после disk ack. redb Immediate/CRC,cache16МиБ,порог4ГиБ. KJN сохранён,backend не смешивать;R2 впереди.

IM2480: эксклюзивный COM,FC03,0009=0,6типизированных каналов/модуль,слоты0–15; резерв100×10 ядра. Missing по модулю,bounded повтор batch. Один управляемый IM-процесс/профиль. PID-mailbox≤32КиБ,статус≤128КиБ. «Модули»: провода/ветвление,поиск1–100,hot config с expected/диском/возвратом,slot*6. Карта и линия — отдельные операции. История64 внеbackup;ДОЛ исходный A/B;записи в прибор нет.

Карты linear-v1/graph-v1,≤256ревизий/128параметров/48КиБ. Provider sensor/manual/calculation; DAG/единицы/возраст проверяются. Каталог81/7наборов,полный59. UI:2списка/группы/поиск,25px/14px;удалённые≤128. Шаблоны≤32/60КиБ,без привязок/введённых значений по умолчанию;библиотека внеbackup,активные карты входят. Глубины ручные,авто-ДОЛ/операций нет.

processing_context строится из committed parameters_stored; окна≤8×302точки,1–300с. Restart/replay сохраняет контекст,новая карта/сессия зависимого источника сбрасывает окно. Календарная регрессия явная; свежесть ядра монотонная. Дешёвые операции общие ядру/processor. Карты/CRC сохранены,float_roundtrip обязателен.

kern-shell обязателен;charts loopback Rust-host/ключ/uPlot,Edge/Chrome. Формы встроены,web SDK впереди. UI:только⋮по контексту,3ползунка;presentation-settings.json с revision внеbackup. Плитка7кеш/хеш,сетка5/5. Трей:импульс толькоackraw,метр5с отдельно отIPC,X скрывает;TRAY.md. LAN/роли:NETWORK_PLAN.

CSV/backup независимы. Backup hourly-v2:raw полностью,params10с+CRC-рецепты,файл/час или новая карта с причиной,папка/сутки. HeaderState≤16МиБ,блок≤1МиБ/target256КиБ,Zstd1;disk.tail+footer;v1 читается. Папка/revision/hot,догон/rollback;BACKUP.md. Restore redb baseline+исходныеseq,ArchiveBaseline48КиБ/CRC. Трей:коллекция/внешний том. Drain/CSV-периоды впереди. Надзор:3старта/60с→safe;lease15с/нет прогресса20с;stop запрещает restart.

Канон:PARAMETERS/IM2480/PROTOTYPE/CORE_VALIDATION. 60Rust/13Python+18native,7hourly+8legacy/6pipeline/28editor+9charts. ПрежниеCOM/UI:CORE_VALIDATION. Стенд:checkpoint/drain,карта/линия/история сохранены,5модулей/30каналов. Не приняты: физика,Win10/слабыеПК/macOS/GPU/24–72ч/power-loss. Далее месячный объём/J4125,drain/ротация основной базы. ASCII пути; AGENTS/MATERIALS.
