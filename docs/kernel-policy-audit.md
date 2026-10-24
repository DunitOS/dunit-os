# Аудит механизмов ядра и политики userspace перед M7

Основание: задача из `task.txt`, этапы 1 и 3. Срез исходного состояния —
`36e85ce`; **все номера строк в таблицах относятся к этому коммиту**, а не к
изменяющемуся рабочему дереву. Для проверки: `git show 36e85ce:<путь> | nl -ba`.
Аудит выполнен чтением кода, поиском объявлений, вызывающих и потребителей;
наличие реализации или marker-файла само по себе не считается результатом QEMU.

Серьёзность: **высокая** — нарушение контракта, утечка ресурсов или
гарантированный отказ поддерживаемого сценария; **средняя** — неправильная
политика, диагностика, потеря данных UI; **низкая** — документация/мёртвые данные.
Статус «правка подготовлена» означает изменение рабочего дерева, которое ещё
требует итоговой проверки. Протоколы, capabilities, формат ФС и ABI этим
документом не меняются. Итоговые результаты M6 фиксируются в `docs/m6-audit.md`.

## Exec, приложения и окружение

| Файл:строка в 36e85ce | Серьёзность / наблюдение | Общий механизм вместо специального случая | Статус аудита |
| --- | --- | --- | --- |
| `kernel/src/command.rs:37–65`, `:106` | Высокая: kernel-terminal ищет bare name по compiled `/system/bin`, `/app`. Сам поиск — shell policy, хотя системный порядок правильный и CWD не добавляется. | Userspace resolver получает PATH из окружения/системной конфигурации, нормализует явные absolute/relative paths; kernel выполняет готовый путь. Kernel recovery-terminal может иметь явно обозначенный bootstrap resolver, но не становиться источником desktop policy. | Подтверждено кодом; нужно проверить единый порядок во всех shell/spawn сценариях. |
| `kernel/src/syscall/mod.rs:353`, `:2501`, `:2537`, `:2865–2883` | Высокая: spawn/PTY имеют второй resolver только `/app`, отличающийся от kernel-terminal. Наличие `/` переводит на буквальный путь, иначе неожиданно меняется семантика. | Один userspace resolver; syscall принимает разрешённый путь. Удаление поиска из kernel должно сопровождаться переводом всех userspace callers, не поломкой bare-name CLI. | Подтверждено; изменение публичной семантики требует осторожной проверки совместимости. |
| `userspace/system_apps/dsh/src/main.rs:154–169` | Высокая: ручной `/app/<name>` ломает `exec /tmp/x`, относительные пути и не обеспечивает общий поиск/передачу аргументов. | Общая библиотека разбора команды и PATH resolution; argv/env передаются существующим native spawn. Системные каталоги прежде пользовательских, CWD только при явно указанном пути. | Подтверждено; исправление userspace готовится в основной задаче. |
| `kernel/src/elf/mod.rs:170` | Средняя: default `PATH=/app` расходится с kernel-terminal и не включает `/system/bin` для tcc/Lua. | Bootstrap окружение должно совпадать с системной конфигурацией userspace; явно переданное окружение сохранять. Не подменять пользовательский PATH скрытым поиском в syscall. | Подтверждено; проверять пустое/отсутствующее/переопределённое окружение. |
| `userspace/dwm_settings/src/lib.rs:969–995`, `:1190–1290` | Средняя: нормальный dock/launcher уже читается из `[application.*]`, `[dock]`, `[launcher]`, а fallback registry содержит шесть compiled приложений. Это не directory scan, но второй источник app policy при missing/garbage config. | Манифест — источник списка и имён приложений; last-known-good сохраняет прежний registry. Bootstrap fallback и его диагностический статус должны быть явно обозначены. | Нормальный manifest-driven путь подтверждён; compiled fallback не считать полной готовностью app-model M7. |
| `kernel/src/lib.rs:1267–1283` | Информация: bootstrap запускает init; это допустимая точка входа ОС. Но вызов по bare `init` зависит от kernel PATH resolver. | Зафиксированный boot entry или утверждённый boot manifest; состав и restart policy сервисов остаются у userspace init. | Не путать bootstrap init с hardcoded списком GUI-приложений. Выбор нового boot-контракта не входит в ремонт M6. |

Проверки поиска должны включать запуск `/tmp/x`, `./x`, относительного пути из
другого каталога, bare-name tcc/Lua, пробелы/аргументы, отсутствующую команду,
каталог вместо ELF, системный и пользовательский одноимённые файлы. Простого
успешного `exec hello` недостаточно для доказательства исправления всех callers.

## Статистика, терминал, GUI и Lua

| Файл:строка в 36e85ce | Серьёзность / наблюдение | Общий механизм / безопасное исправление | Статус аудита |
| --- | --- | --- | --- |
| `kernel/src/syscall/mod.rs:2156–2176`; `userspace/system_apps/dufetch/src/main.rs:100–104`, `:148–160`; `userspace/system_apps/gui_server/src/main.rs:4720–4721`, `:4983–4984` | Сообщённое «RAM 99% против 37%» **не подтверждено разными формулами**: оба UI используют `SystemStats.pmm_used_bytes / pmm_total_bytes`; dufetch выводит те же bytes. Heap — отдельная строка. | Сопоставлять одновременные samples одного boot, объём RAM, число/размер mappings и инвариант used+free=total. PMM total — учитываемая аллокатором память, а не обязательно весь заявленный QEMU RAM. Не заменять PMM на heap для красивого процента. | Источник един по коду. Разные моменты времени и обнаруженные buffer leaks могут менять реальное потребление; конкретная причина исходных двух процентов не доказана. |
| `userspace/system_apps/dsh/src/main.rs:334–338`; `kernel/src/shell.rs:1215–1217` | Средняя: uname-подобные ответы содержат идентичность/версию в shell, а не авторитетные kernel facts. Сам dufetch уже получает доступные счётчики через syscall; нельзя объявлять весь его вывод выдуманным. | Согласовать uname/sysinfo источник фактов. До решения показывать доступные измеренные facts и честно обозначать unavailable; не придумывать версию/железо. | Найдено расхождение источников. Новый identity ABI не реализуется без решения человека. |
| `userspace/system_apps/gui_terminal/src/main.rs:162`, `:669–686` | Средняя: число колонок основано на округлённом TTF advance, но длинный цветовой run рисуется с естественным дробным advance. Дрейф может обрезать конец строки и рассогласовать курсор. | Одинаковая целочисленная клеточная сетка для glyph и cursor; ANSI/UTF-8 поток сохраняет состояние между чтениями. | Подготовлена отдельная terminal-model правка; нужны host tests и реальная отрисовка в QEMU. |
| `userspace/system_apps/gui_terminal/src/main.rs:420–445`, `:970` | Средняя: уменьшение ширины усекало строки без reflow; grid менялся до успешного выделения нового surface. | Soft-wrap boundaries отделять от hard newline, сохранять содержимое при reflow; candidate grid принимать только после успешного surface preflight. | Подготовлена правка модели; нельзя считать исчезновение одиночного символа доказанным одним code review. |
| `kernel/src/syscall/mod.rs:1771–1783` | Средняя: декодирование kernel-terminal output отдельно на каждом write может повредить UTF-8, если один code point разбит между вызовами. | Stateful incremental decoder в конечном terminal stream, с явным поведением malformed sequences. | Отдельная проверка от GUI-terminal; не смешивать два terminal backend. |
| `kernel/src/process/mod.rs:64`; `userspace/system_apps/gui_settings/src/main.rs:1008–1013` | Высокая: kernel shared-object cap 16 MiB, клиент допускает 64 MiB. Surface 3072×1920×4 = 23 592 960 bytes гарантированно отклоняется независимо от RAM. | Общий bounded memory-object mechanism с совместимым размером; сохранять allocation rollback при OOM, выводить errno/bytes. Не уменьшать визуальное разрешение ради теста. | Подготовлено повышение общего лимита до 64 MiB; требуется native проверка resize/OOM. |
| `kernel/src/process/mod.rs:796–799`, `:865–867`, `:1078–1090`; `userspace/system_apps/gui_server/src/main.rs:796–803`, `:1119`, `:3653` | Высокая: `handle_close` не unmap; page refs живут отдельно. Server перезаписывал mapping/handle при новом announcement и не снимал mapping при reap. Реальная утечка физических страниц. | Явный владелец `(address, length, handle)`; `vm_unmap` затем close при replacement/failure/reap. Длину брать из kernel, не из недоверенного announcement. | Подготовлена общая `libdunit::shared_buffer::release`, cleanup server и GUI clients; runtime acceptance ещё обязателен. |
| `userspace/system_apps/gui_settings/src/main.rs:1031–1038`, `:1091`, `:1306`; аналогичные resize в client/calc/files/demo/terminal | Высокая: fresh mapped buffer на error и old mapped buffer на success закрывались без unmap; failed transfer оставлял duplicate в ряде клиентов. | Освобождать обе локальные ссылки; preflight failure оставляет прежние geometry/mapping. Remote mapping независим от local mapping lifetime. | Подготовлены исправления всех перечисленных клиентов. Проверять многократные resize и close/open, а не один переход. |
| `userspace/system_apps/gui_settings/src/main.rs:1078–1094` | Средняя: успех очереди `ipc_send` ещё не означает принятие ACK/IMPORT/ATTACH/COMMIT сервером. Частичная отправка не откатывается восстановлением пары локальных полей. | Полноценная клиентская state machine существующих RESULT/CONFIGURE/FRAME_DONE, без изменения wire. | Известное ограничение; локальный lifetime fix не объявлять полной transactional-реализацией всего GUI protocol. |
| `userspace/dwm_settings/src/lib.rs:1056–1113`, `:1152–1161` | Средняя: `applied` увеличивается даже при неизвестном key/invalid value; `[theme] garbage=42` может стать `valid`. | Диагностика section/key/line; checked parser/overlay и last-known-good. Не считать число строк числом применённых значений. | Подтверждено; строгий `overlay.rs` уже существует, основной load требует интеграции. |
| `userspace/dwm_settings/src/lib.rs:1232`, `:1244–1248`, `:1268` | Средняя: registry unknown keys игнорируются, bad bool становится false, workspace clamp, неизвестные ID удаляются без предупреждения. | Проверка registry целиком, явные warnings; не терять работающий registry при плохом reload. | Подтверждено; диагностика scalar parser не закрывает автоматически app registry. |
| `assets/dwm/themes/green-tea.toml:22–24`, `:40–46` | Низкая: комментарий `max_windows` перед effects при отсутствии поля; документация silent ignore и font fallback не соответствует checked overlays/реальному load. | Комментарии описывают фактически присутствующие секции, поддерживаемые форматы и ошибки. | Подтверждено; исправление документации входит в задачу. |
| `userspace/system_apps/dwm_lua/main.c:140–165`, `:232`; `userspace/lua_runtime/limits.h:48`; `userspace/system_apps/gui_server/src/lua_policy.rs:189–257` | Информация: DWM VM уже ограничен 100 000 инструкций, Lua-owned heap 1 MiB, source 64 KiB; отдельный процесс, deadline 50–5000 ms, output ≤16 KiB. Compile и произвольный C-call не прерываются count-hook. | Сохранять process isolation и monotonic deadline; не выполнять Lua синхронно внутри compositor. Lua heap quota не называть kernel process-wide quota. | Механизмы подтверждены кодом и существующими regression cases; текущая runtime проверка нужна после правок. |
| `userspace/system_apps/gui_server/src/main.rs:4556`, `:4613–4623` | Средняя: Lua failure только serial; старые tiled states сохраняются, нет desktop notification и rollback на встроенное размещение. | Единый failure path: cancel, clear expected/motion, восстановление сохранённого floating state существующим WM mechanism, config-gated toast. | Подтверждено; failure UI/fallback реализуется в основной задаче. |
| `Makefile:196–200`; `third_party/lua/src/linit.c:29–40`; `userspace/lua_runtime/libraries.h`; `config/lua.h` | Информация: standalone Lua и DWM runner имеют разные назначения. Standalone экспортирует io/os/package/debug; DWM — pure libraries, без io/os/package/debug/coroutine и file loaders. | Не ломать полноценный CLI interpreter ради sandbox: все его операции идут через обычные native process syscalls. Проверять sandbox отдельно. | Не обнаружен Lua-specific privilege bypass; это не утверждение о полной готовности будущей app permission model. |
| `third_party/lua/src/liolib.c:75`; `:290`; `third_party/lua/src/loadlib.c:210–230`; `third_party/lua/src/loslib.c:143` | Информация: standalone `io.open` реален; `io.popen` при текущей ISO-C сборке явно unsupported; `package.loadlib` явно отказывает без dynamic loader; `os.execute` делегирует libc system. | Честные ошибки неподдержанного, никаких фиктивных success. os.execute/system и spawn attributes требуют отдельного libc контракта. | Не считать наличие функции доказательством работающего popen/dlopen. |
| `toolchains/dunit-musl/src/process/posix_spawn.c:21–22` (submodule, закреплённый в 36e85ce) | Высокая: file actions и attributes молча игнорируются; system/popen не могут полагаться на POSIX semantics. | Реализовать поддержанный subset либо явно ENOSYS/ENOTSUP для непустых неподдержанных действий/флагов. | Исправляется в musl fork отдельным коммитом и бампом; сеть не требуется. |

## Дополнительный обзор ядра: механизм, bootstrap и оставшаяся политика

Поиск охватывал `kernel/src`: строки путей, имена приложений, launch/resolver,
константы размеров, TODO/FIXME/stub, embedded GUI данные, capability и VM пути.
Это обзор архитектурных рисков, **не доказательство отсутствия всех ошибок** в
ядре и драйверах; hardware validation за пределами QEMU не выполнялась.

| Файл:строка в 36e85ce | Оценка | Общий механизм / статус |
| --- | --- | --- |
| `kernel/src/fs/vfs.rs:578`, `:631–632` | Низкая: kernel-seeded `/cfg/gui/shortcuts.conf` содержит desktop shortcuts; поиск потребителей не находит reader. Мёртвый второй источник policy. | Удалить seed и константу после проверки всех ссылок; реальные shortcuts уже в userspace DWM config. |
| `kernel/src/fs/vfs.rs:638–642` | Средняя: kernel знает путь и содержимое desktop session store. | Создание session state относится к userspace/образу. Перенос учитывать protected system roots и MemFS persistence; не обходить защиту новым special-case syscall. |
| `kernel/src/syscall/mod.rs:1356` | Не «случайный hardcode»: `PROTECTED_ROOTS` — действующая граница защиты system/virtual trees. | Не удалять и не расширять права в ходе чистки путей. Будущая manifest/capability-based FS policy требует человеческого решения и denial tests. |
| `kernel/src/lib.rs:191`, `:374–378` | Kernel recovery terminal palette и boot background — оставшаяся presentation policy. Boot экран не является вторым runtime compositor; bootstrap требует собственного диагностического UI до userspace. | Не смешивать boot/recovery и desktop; будущий вынос recovery-shell отдельно планировать с сохранением возможности диагностики failed init. Не удалять boot asset без эквивалентного boot поведения. |
| `kernel/src/drivers/display/bochs.rs:31–35` | Информация: candidate modes фильтруются по hardware/VRAM. Это перечень возможностей driver, не выбор desktop theme/resolution. | Выбранный пользователем режим остаётся в userspace config; device capability discovery не переносить в policy TOML. |
| `kernel/src/drivers/block.rs:7–10`; `kernel/src/drivers/virtio_blk.rs:10`; `kernel/src/drivers/registry.rs:111` | Имена device nodes, sector sizes, register offsets и protocol magic — механизм/формат, сами по себе не app-policy hardcode. | Проверять bounds/endianness/device identity; не делать configurable каждый hardware constant. RAM block device reference backend не считать persistent root. |
| `kernel/src/drivers/usb/xhci.rs:199–210` | Средняя, отдельный review: nonstandard version принимается при plausible capability fields. Это compatibility heuristic, а не подтверждение USB stack readiness. | Зафиксировать поддержанные устройства/версии и тесты; не объявлять hardware exception универсальной совместимостью. Не переписывать USB в M6 без воспроизводимого demand. |
| `kernel/src/syscall/mod.rs:321–327`; `kernel/src/process/mod.rs:31`, `:64` | Bounds copy/path/fd/object — необходимые resource limits. Проблема возникает при рассогласовании механизмов с обещанным сценарием, как surface 3K. | Документировать bounds, проверять boundary/overflow/OOM; менять generic cap вместе с acceptance, а не обходить per-app исключением. |
| `kernel/src/sync.rs:8–15`; `kernel/src/process/mod.rs` | Формулировки о готовности spinlocks к SMP не доказывают SMP всей ОС. Процессная таблица и execution model остаются UP. | Сохранять честное UP описание; full SMP не входит в доведение static musl. |
| `kernel/src/fs/vfs.rs:609–647` | Root реально MemFS с initrd; наличие DunitFS `/persist` и installer не означает disk-root/installed boot. | Отмечать M5 частичным до реального persistent system/data root, запуска init с диска и reboot verification. Не переименовывать MemFS в persistent root в документации. |

## Решения человека перед M7

Ниже только варианты и рекомендации. Этот аудит не утверждает новые ABI,
manifest format или permission policy.

| Решение | Варианты и trade-offs | Рекомендация / что требуется до реализации |
| --- | --- | --- |
| Единое ожидание wait-set | Несколько polling APIs проще локально, но дают busy loops и сложную отмену; wait-set объединяет IPC/input/timers/process readiness, но фиксирует новый ABI и lifetime semantics. | Сначала согласовать типы объектов, права, deadline, cancellation, readiness/edge semantics; затем versioned ABI + contract tests. Это первый блокер event loop M7. |
| SDK runtime | `no_std+alloc` поверх libdunit использует существующие DUI/DSS и меньше POSIX surface; Rust std поверх musl открывает экосистему, но требует полноценного Dunit std backend и большего POSIX subset. | Начать с no_std+alloc с изолированными backend interfaces; окончательный выбор принимает человек до фиксации SDK API. M8 нельзя молча объявлять уже готовым из-за musl. |
| App manifest | Минимальный TOML schema проще использовать существующим tooling; полный package/capability manifest сразу уменьшает будущие миграции, но преждевременно фиксирует app-model. | Утвердить минимальные id/name/icon/kind/entrypoint/schema_version и способ запроса прав; один source для launcher/dock/init, проверка unknown fields и last-known-good. Существующий DWM registry не равен принятому dunit.toml контракту. |
| Drag-and-drop | Передача текста/пути удобна для CLI, но путь не предоставляет и не ограничивает доступ; handle/capability ограничивает права и lifetime, но требует принятого протокола и UX отказов. | Text receiver получает экранированный текст/путь; file operation — ограниченный handle. Согласовать source/target consent, lifetime, cancellation и доступ к файлам до реализации. |
| `tcc -run` и исполняемая память | Компиляция ELF в файл + spawn уже укладывается в процессную модель; in-memory execution может использовать существующий RW→RX mprotect, но TCC relocation, alignment и W^X transition требуют проверки. | **Новый syscall заранее не нужен**: `sys_mprotect` уже есть (`syscall/mod.rs:1835–1847`), W+X запрещён (`:1840`), `Process::protect_range` меняет page permissions. Проверить TCC backend RW→RX на отдельном примере; не объявлять `-run` готовым по наличию mprotect. Выбор режима/новых прав остаётся человеку. |
| Источник identity для dufetch/uname | Read-only build/system metadata можно versioned file; uname/sysinfo syscall делает kernel facts авторитетными, но меняет ABI; userspace branding TOML подходит оформлению, не measured kernel identity. | Разделить branding и факты. Рекомендация — versioned kernel identity API после согласования состава и ABI; до этого не выдумывать недоступные значения и сохранять текущий SystemStats для измеряемых counters. |

## Что должна доказать итоговая проверка

- Терминал: ANSI/UTF-8 на границах write/read, длинные строки, cursor alignment,
  shrink/grow reflow без потери текста и сохранение прежнего grid при OOM.
- Resize: максимум поддерживаемого 3K, повторные maximize/restore и close/open;
  выделение сверх cap, недостаток памяти, cleanup после отказа. Прогоны при
  разных QEMU RAM сравниваются по фактическим bytes/errno/PMM samples, не только
  по строке «previous surface retained».
- Lua: корректная политика, syntax/runtime/quota/memory error, долгий runner,
  invalid output, отсутствие частичной политики, отзывчивый desktop и видимый
  fallback/notification. Standalone file I/O проверяется отдельно от sandbox.
- Config: unknown key, invalid color/boolean/range/schema, malformed registry,
  плохой reload сохраняет last-known-good с явной диагностикой.
- M6: строгие ошибки unsupported libc действий, файлы/pipes/spawn/thread/TLS,
  on-device tcc/Lua; сеть/DNS/socket readiness остаются M8.

Все runtime проверки выполняются только через `tools/qemu_test.py`; объём RAM,
marker-file и результаты записываются в итоговый M6 отчёт. Этот read-only аудит
не выдаёт будущие прогоны за уже выполненные и не закрывает M7 решения человека.

## Результат исправлений

Таблица выше сохраняет координаты исходной базы `36e85ce`. Текущий
проверенный результат описан в [m6-readiness.md](m6-readiness.md): поиск
команд вынесен в userspace, kernel принимает буквальный путь; GUI bootstrap
передаёт `/app/init`. Приведённые в таблице старые строки не являются
координатами исправленного кода. Не все policy findings требуют удаления:
bootstrap entry, механические границы памяти и protected storage mounts
остаются механизмами; шесть архитектурных решений M7 не принимались.
