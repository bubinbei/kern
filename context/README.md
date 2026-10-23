# Короткий рабочий контекст

`python tools/project_context.py list` → `start core`: STATE и 4 последних раунда. Поиск: `search core "вопрос"`; полная история с `--archive`. Не загружать все документы. Без Python открыть obsidian/Projects/core/STATE.md и RECENT.md.

После задачи подготовить локальный JSON: id (ASCII/kebab-case), date (ISO с часовым поясом), summary (1–2 предложения, ≤750 символов), validation (≤350), sources (1–4 существующих Markdown внутри репозитория), next (≤200, необязательно). Команда: `python tools/project_context.py record core --input context/round.local.json`; затем check. STATE≤2500, RECENT≤6500; RECENT не редактировать вручную, полные rounds не сокращать. Повтор ID с другим содержимым запрещён.

При прерывании между записью истории и RECENT выполнить refresh/check. context/.record.lock удалять только убедившись, что нет активного писателя. Полные source-хеши относятся к своей ревизии, не обновлять прошлые раунды. Скрипт использует только стандартную библиотеку Python.
