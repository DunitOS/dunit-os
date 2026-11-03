# M7.7: системный clipboard и drag-and-drop

## Контракт

Посредник — `gui_server`. Он проверяет PID из ядра, фокус для операций
clipboard и реальный захват указателя для начала drag. Политика desktop не
перенесена в ядро. Старые GUI-клиенты работают без transfer capability;
новый канал включается согласованным `FEATURE_DATA_TRANSFER` (`1 << 2`).

`XFR1` — 32-байтный little-endian конверт: magic, kind u16, reserved-zero u16,
request u64, transfer u64, arg0 u32, arg1 u32. Типы, действия, статусы и
операции файлов задаются в `abi/transfers.abi`; `tools/gen_abi.py` генерирует
Rust и C значения. Кодеки и конечный автомат находятся в `gui_protocol_v1`.
Большие данные передаются через kernel-attested handle, затем ограниченно
копируются в частную память получателя. Чужой номер handle в пакете не
является полномочием.

Представления: UTF-8 text, нормализованный URI list, Files, RGBA и собственный
MIME. Максимум 16 форматов, 256 файлов, 4096 байт пути, 16 MiB на blob и
32 MiB суммарных payload-копий активных чтений clipboard. Некорректные длины, UTF-8, RGBA размеры,
повторные id и неподходящие состояния отвергаются. Поток одного получателя
сохраняет порядок; его заполненная IPC-очередь не блокирует остальных.

DnD запрашивает данные только после принятого drop. Перемещение между
рабочими столами сохраняет захват источника. Escape, потеря процесса,
неподходящая цель и timeout завершают операцию и освобождают escrow.
После отпускания указателя broker допускает запоздавший ответ уже выбранной
цели до deadline; переместить такую операцию на другую цель нельзя.
Clipboard материализует все представления до ACK, поэтому Copy переживает
выход источника. Cut файлов не удаляет источник до успешного Move.

## Файловые полномочия

Одобрена гибридная модель: текстовому получателю передаётся экранированный
путь, файловому — настоящий ограниченный capability. Syscalls 91–94:
FileCapOpen, FileCapOp, IpcSendHandle, IpcReceiveHandle. Получатель заранее
включает attached IPC; обычный receive не поглощает пакет с вложением.
Передача атомарна, неудачная отправка откатывает дубликат. Нужен TRANSFER;
передаваемые права могут только сужаться.

Capability привязан к открытому объекту backend, не к строке пути.
Относительный lookup запрещает обход через `..` и не выходит из области.
MemFS сохраняет открытый удалённый файл; DunitFS инвалидирует удалённый inode,
не перенаправляя старый descriptor на повторно использованный слот.
Получатель Files получает READ|TRANSFER; исходный WRITE для Move остаётся
у broker. Реальный Move — атомарный no-overwrite rename в одном backend.
Между backend Move возвращает EOPNOTSUPP, без copy-unlink fallback.
Виртуальный VFS-корень `/` не является одним backend: для файловой передачи
нужен настоящий каталог, например `/tmp` или `/persist`. Directory listing
сохраняет существующий ABI: максимум 1024 записи за чтение и 64 UTF-8 байта
на имя; более длинное имя возвращает ошибку, а не усечённое полномочие.
Copy выполняется общей ограниченной задачей: один блок 4 KiB или один узел
за poll, максимум 64 уровня, 16384 узла, 256 MiB. Отмена удаляет только
созданные этой задачей объекты по identity, по одному за poll.

Действия Copy/Move/Link согласуются явно. Встроенные файловые приложения
поддерживают Copy/Move; Link не изображается копированием: файловая система
не предоставляет создание ссылок. Custom-получатель может согласовать Link.
Capabilities имеют обычный срок жизни handle с RAII, а не kernel lease:
завершение transfer освобождает broker escrow, но не отзывает уже полученный
handle у клиента. Клиент может удерживать только выданные ему права.
Это ограничение переданного полномочия, а не новый per-process sandbox ФС:
существующие ambient path-based syscalls сохраняются. Файловые приёмники
этого слоя выполняют операции через полученные caps, не восстанавливая права
из строковых путей.

## SDK и приложения

`Window` имеет единственный authenticated IPC pump; transfer-пакеты не теряются
во время FrameDone. Typed SDK предоставляет общие payload/client/session API.
Поля ввода выполняют controlled update, привязывают отложенные операции к
точному editor/focus/revision, вставляют paste одной undo-операцией; readonly
можно копировать, password нельзя экспортировать. Коллекции экспортируют
выделенные подписи/TSV. Для text DnD начало внутри выделения отличается от
обычного выделения мышью.

Files использует системный clipboard, множественное выделение, capability
на выбранный destination, Copy/Move и отменяемую фоновую копию. Полученные
директории не дают ambient fallback. Terminal вставляет текст и корректно
экранированные пути через общую shell-грамматику. Settings поддерживает
clipboard/DnD полей и открывает переданный TOML как draft с явным Apply.
Calc экспортирует readonly результат. Dock выбирает opener по `open_formats`,
запускает процесс только после drop и передаёт capability точному PID после
его регистрации. Политика не зависит от названия приложения.

## Приёмка

`tools/dnd_matrix.json` задаёт все пары production GUI-приложений. Добавление
приложения в desktop без строки матрицы валит `test_dnd_matrix.py`.
`test_sdk_gui_fixture.py matrix` запускает настоящие бинарники под отдельными
metadata identities и проверяет clipboard/файлы/результат shell получателя,
а не только доставку события. `transfer` проверяет независимые процессы,
ленивый DnD, Escape/неподходящую цель, RGBA/custom MIME, Link, переход между
рабочими столами, file identity и dock; `clipboard` — controlled поля и text DnD,
UTF-8, undo, readonly, password и сохранение после выхода источника.
`m6_handle_markers.json` проверяет native права, IPC rollback, реальное
копирование и ограниченную отмену. Отдельные prepare/verify контракты
проверяют DunitFS capabilities и Sync через перезагрузку частного диска QEMU.
Все запуски ОС идут через `tools/qemu_test.py`.


Дополнительные native сценарии матрицы:

```sh
DUNIT_MATRIX_CASES='gui_files->gui_files' DUNIT_MATRIX_MOVE=1 \
  python3 tools/test_sdk_gui_fixture.py matrix --accel tcg
DUNIT_MATRIX_CASES='gui_files->gui_terminal' DUNIT_MATRIX_QUOTED=1 \
  python3 tools/test_sdk_gui_fixture.py matrix --accel tcg
```

Первый проверяет два настоящих Move и отсутствие обоих исходников, второй —
путь `файл 'quoted'.txt` через Terminal и настоящий shell. Эти сценарии
требуют нового образа; `--reuse-image` предназначен для неизменного overlay.
Матрица отправляет явные make/break клавиш: таймер отпускания `send-key`
использует виртуальное время QEMU и не является барьером обработки на TCG.
Перед сменой фокуса подготовка выделения дожидается реального ACK clipboard;
каждый запуск окна дожидается committed размера, а не только первого кадра.


## Итог проверки: 3 ноября 2026

449 Rust-проверок и 33 Python-проверки прошли. В QEMU TCG (512 MiB, один vCPU)
зелёные все 49 пар приложений и дополнительные native контракты:

- transfer: 17 обязательных маркеров, включая 40960 байт UTF-8, RGBA/custom,
  Link, workspace transitions, Escape/unsupported, source exit, file identity, dock;
- clipboard: 13 маркеров controlled paste/text DnD/cut/undo/readonly/password;
- production multi-file Move и экранированный путь в настоящем shell;
- handle/file-capability проверки: 7 маркеров; DunitFS prepare + reboot verify:
  3 + 3 маркера, только на отдельной копии тестового диска;
- регрессии Input: 20 маркеров; полный GUI smoke: 30 маркеров, включая
  старых клиентов, resize/maximize/restore, notification и recycling;
- обычный `cargo dunit run` для GUI-проекта через desktop composition завершился
  `[GUI-EXIT] ... kind=0 code=0`.

Во всех этих native контрактах запрещённые panic/failure маркеры отсутствуют.
Маркеры и существующие проверки не удалялись ради прохождения; ожидания GUI
привязаны к фактической готовности, а не к числу кадров.
