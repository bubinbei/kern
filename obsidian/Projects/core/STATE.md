---
project: core
type: state
status: active
updated: 2026-10-25
---

# KERN — сквозной прототип 0.1.0

Rust1.96/Windows11. IM2480→redb→processor→параметры→uPlot/CSV/backup. Ядро единственный писатель, публикация после disk ack. redb Immediate/CRC,cache16МиБ,порог4ГиБ. KJN сохранён, backend не смешивать; R2 впереди.

IM2480: эксклюзивный COM,FC03,0009=0,6типизированных каналов/модуль,слоты0–15; резерв100×10 ядра. Missing по модулю,bounded повтор batch. Один управляемый IM-процесс/профиль. PID-mailbox≤32КиБ,статус≤128КиБ. «Модули»: провода/ветвление,поиск1–100,hot config с expected/диском/возвратом,постоянные slot*6. Карта и линия — отдельные операции. История64 локальная,не в JSONL. ДОЛ только исходный A/B; записи в прибор нет.

Карты linear-v1/graph-v1 immutable,≤256ревизий/128параметров/48КиБ. Provider sensor/manual/calculation; DAG/единицы/возраст проверяются. Каталог81/7наборов,полный59. Используемые/неиспользуемые:группы/поиск,строки25px/14px;удалённые≤128 вUI-сессии. Шаблоны≤32версий/60КиБ,по умолчанию без привязок/введённых значений; библиотека пока не в backup. Активные карты входят. Глубины пока ручные,авто-ДОЛ/распознавания операций нет.

processing_context строится из committed parameters_stored; окна≤8×302точки,1–300с. Restart/replay сохраняет контекст,новая карта/сессия зависимого источника сбрасывает окно. Календарная регрессия явная; свежесть ядра монотонная. Стандартные дешёвые операции общие для проверки ядра/processor; тяжёлого SDK нет. Старые карты/CRC сохранены,float_roundtrip обязателен.

kern-shell обязателен; charts отдельный loopback Rust-host/ключ/локальный uPlot,окно Edge/Chrome. Каталог и Modules пока встроены в host,web SDK впереди. UI:только⋮по контексту,3ползунка;profile/presentation-settings.json с expected_revision,внеbackup. Плитка7кеш/хеш,сетка5/5. Трей:цвет/импульс толькоackraw,место/файлы5с,метр отдельно отIPC,X скрывает;TRAY.md. LAN/роли — NETWORK_PLAN.

CSV/backup независимы,CRC/позиции,restore в новый профиль. Общий drain/периоды впереди. Надзор processor/csv/backup:3старта/60с→safe;lease15с/нет прогресса20с;stop запрещает restart.

Контракт: PARAMETERS/IM2480/PROTOTYPE/CORE_VALIDATION. 51Rust/13Python+18native,8pipeline+15UI,28editor+9charts;17COM/9management+13UI/18core/6старого тракта. Стенд обновлён с checkpoint/drain;карта/конфиг/история сохранены,5модулей/30каналов. Не приняты: физика,Win10/слабыеПК/macOS/GPU/24–72ч/power-loss. Далее config/replay/drain,отказы,периоды. ASCII пути; AGENTS/MATERIALS.
