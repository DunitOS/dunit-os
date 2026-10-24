# Результат M6 и вход в M7

Рабочая база аудита: `36e85ce`. Подробная таблица **каждого** пункта M6,
исходные доказательства и расхождения документов: [m6-audit.md](m6-audit.md).
Находки ядра с файлами/строками и серьёзностью относятся к той же неизменяемой
базе: [kernel-policy-audit.md](kernel-policy-audit.md). Результаты новых
интегрированных прогонов добавляются после их завершения; наличие кода не
означает закрытый acceptance gate.

## Реализованные изменения

- Userspace владеет поиском программ: `/assets/exec-path` задаёт порядок
  `/system/bin:/app`; пустые/относительные элементы PATH не добавляют cwd.
  Ядро запускает готовые пути и больше не подставляет PATH/SHELL/CWD. PTY
  передаёт реальные argv, включая пустые аргументы и пути с пробелами.
- Утверждённый человеком пакет: Fcntl=86, Poll=87, Ioctl=88; версия блока
  SpawnArgs/PtySpawn включается старшим битом длины. Старые ограниченные длины
  не используют шестой регистр, который прежние wrappers не инициализировали.
  Старые вызовы нельзя проверять простым условием `r9 != 0`.
- Descriptor flags CLOEXEC принадлежат fd, APPEND/NONBLOCK — общей open
  description. Spawn-actions применяются к таблице ребёнка до публикации
  runnable-процесса; родительские fd не перенаправляются. Привилегированные
  capabilities через POSIX fd inheritance не наследуются.
- Poll ждёт в kernel wait queue с абсолютным monotonic deadline. Модель
  единого M7 wait-set этим не выбирается. Eager file MAP_PRIVATE сохраняет
  offset файла и W^X; shared/demand/COW/SIGBUS не изображаются реализованными.
- Терминальный ioctl имеет узкий проверяемый native contract. Termios с
  canonical input/echo/signals не реализован и явно отклоняется, как и
  неподдержанные spawn attributes, cancellation и open flags.
- Статический C TLS получает реальные ELF auxv headers. Dead thread освобождает
  kernel stack/TLS после HAL escape; musl join забирает native status, detached
  использует отдельно одобренный ThreadExit v1 auto-reap. Проверены 1000 join
  и 1000 detached в одном процессе с TLS/errno/malloc/yield, без прежнего OOM.
  Планировщик сохраняет единственный Ready-поток, когда его родитель ждёт join.
- ClockGetTime realtime использует UTC boot timestamp Limine и monotonic
  timebase. Отсутствующий timestamp — ошибка, а не uptime под видом UTC.
- MemFS разделяет имя и идентичность открытого файла: unlink скрывает имя,
  открытый файл живёт до последнего close; новое имя не меняет старый fd.
  Курсоры directory snapshots поддерживают rewind/seek cookies.
- Resize освобождает mapping отдельно от capability; лимит shared VM позволяет
  framebuffer до 3K. Терминал переносит и переразмещает строки. Lua failures
  отменяют runner и tiling animation, восстанавливают Floating и уведомляют;
  прежний запрос не перезапускается каждый кадр.
- Главный DWM config, registry, app/widget config проверяются до применения;
  ошибка сохраняет прежние значения и сообщает строку/поле/причину. Registry
  по умолчанию находится в TOML, не в списке программ Rust. Мёртвые kernel
  shortcuts удалены, начальный session-store поставляется обычным asset.
- SDK cross-files, machine-readable provenance/ABI hash, ELF profile gate,
  source/compiler/sysroot cache invalidation и pinned upstream libc-test subset
  входят в обычную сборку. Explicit host headers отклоняются; host include/
  library environment очищается, linker получает настоящий target sysroot.

## Что не относится к M8, но всё ещё открыто

Это **не** утверждение о полном завершении M6 или готовой POSIX-системе.
Secure entropy interface/source отсутствует; TSC bootstrap bytes не подходят
для криптографии. Signals/interruption, dynamic loader/DTV, disk-backed system
root, POSIX ownership/accounts, полноценная termios line discipline, master PTY
geometry publication и замороженная previous-release ABI suite остаются
отдельными gates. Полный M6 нельзя закрывать, называя эти пункты «сетью».

Приватный файловый mmap — eager snapshot с ограничениями EOF; он не является
универсальным mmap для будущего dynamic loader. Directory iteration — snapshot
с явным overflow; настоящий fd-backed directory object ещё отсутствует.
Readiness для файлов/pipe/PTY не заменяет согласование M7 wait-set с IPC/timers.
Корень остаётся MemFS, DunitFS v2 — только `/persist` (M5 не закрыт).

## Чистые зависимости от M8

Socket objects/rights, TCP/UDP/IP, packet RX/TX, DNS resolver и network readiness
требуют сетевого milestone. Их не реализовывали Linux-shaped syscall remap.
HTTPS дополнительно требует secure entropy и TLS library, то есть блокируется
не только M8. Эти различия сохранены в таблице M6.

## Шесть решений человека перед M7

| Решение | Варианты, плюсы и минусы | Рекомендация |
|---|---|---|
| Wait-set | Один bounded native wait для IPC/timer/fd: справедливый event loop, но новый ABI. Раздельные waits: проще, но усложняют общий loop. | Сначала согласовать object types, deadline и wake races. |
| Runtime SDK | no_std+alloc: узкий native контракт, меньше требований к libc. std поверх musl: шире ecosystem, но требует полноценного backend. | Первый SDK на no_std+alloc; поддержку std не обещать. |
| App manifest | Общая versioned schema даёт один registry/init/package source; раздельные форматы проще локально, но расходятся. | Согласовать общую schema/capabilities и только затем реализовать. |
| Drag-and-drop | Пути просты, но дают ambient authority и гонки. Scoped capabilities точнее ограничивают доступ, но требуют lifetime/revocation. | Передавать ограниченные handles после выбора модели drop. |
| tcc -run | Файл+spawn уже работает и изолирует. In-memory relocation быстрее, но требует корректных страниц, RW→RX и ownership/lifetime. | Существующий mprotect достаточен как механизм; выбор execution/JIT policy остаётся человеку. Не разрешать W+X. |
| dufetch identity | Конфиг подходит для имени дистрибутива; native query — для kernel/hardware facts. Новый uname/sysinfo требует утверждённого ABI. | RAM брать из общего SystemStats; согласовать источник остальных полей. Fake uname удалён, новый источник не выбран. |

Первые шаги M7: принять эти решения; определить versioned SDK release/profile;
собрать простой native example через generated cross-files; проверить mixed
wait loop; затем единый UI/component и packaging flow. Известные gaps выше
должны оставаться явными prerequisites соответствующего SDK профиля.

## Проверки 2026-10-24

- Native thread stress: 1000 create/join + 1000 detached, оба маркера OK,
  процесс завершился с кодом 0 (`m6_thread_stress_markers.json`).
- Настоящий DunitFS `/persist`: файл 131072 байта записан через musl,
  VM остановлена, тот же установленный raw image загружен заново; readback
  подтвердил hash `b4c071280fb62325` (`m6_persist_*_markers.json`).
- Предыдущие интегрированные прогоны: core (native fd/PTY argv/libc-test/Lua
  и on-device TCC rebuild) прошёл; большой regression дал 33 зелёных контракта,
  после исправлений отдельный remaining-прогон подтвердил ещё четыре
  (fileops/hardening/setup/themes). Это результаты нескольких прогонов, а не
  утверждение об одном зелёном составном запуске.

- Свежий regression после lifecycle fix: freestanding C ABI (реальные
  AT_PHDR/PHENT/PHNUM/ENTRY), pthread mutex/join, cond broadcast и static
  profile/barrier/pipe/directory contracts прошли вместе. Raw thread_test
  подтвердил старый отрицательный exit code, EINVAL для auto-reap main,
  исчезновение detached status и прежний fault isolation/XMM contract.
- Offline пересборка sysroot из чистого fork после удаления generated source
  fingerprint воспроизвела `libc.a` побайтово:
  `3941cd27e27ff606698b8329a2d290c7db06afc8c9bd0b8ee78cf528be1874fa`.
- Lua error и runner timeout: оба QEMU contracts прошли, desktop ready и
  notification queued=1 получены. При смене topology запрос может быть новым;
  отсутствие retry storm для неизменного topology проверяется host test.

- GUI smoke полностью прошёл на 512/768/1024 MiB: maximize/restore,
  Terminal/Files reconfigure, Settings live mode switch, close/relaunch;
  resize allocation/map failure diagnostics отсутствуют. Root cause прежнего
  retained-surface случая: capability close не освобождал mapping, а лимит
  shared VM 16 MiB исключал 3K; исправлены оба lifetime/limit механизма.
  Это не доказывает полную transactional RESULT-ack модель GUI: она остаётся
  отдельным протокольным улучшением, wire format не менялся.
- Production 3K contract прошёл: Settings clicks переключили 3072x1920,
  2880x1800, 2560x1600, 2560x1440 и 1920x1080; screendump dimensions
  совпали. После удаления kernel PATH bootstrap init также переведён на
  явный `/app/init`, что проверено настоящей GUI-загрузкой.

- Production SVG UI contract прошёл: смена пресетов, SVG wallpaper,
  shell/Files icons и реальные клики. Наличие полного icon archive не означает
  поддержку всех SVG-фич; статический SVG профиль остаётся документированным.

## Итоговый интегрированный gate

После всех исправлений свежая offline сборка и один составной QEMU-прогон
`tools/m6_suite.py --group regression --build` прошли полностью:
**40 полных контрактов, 73 команды, 175/175 required markers, 0 forbidden**.
Включены original file/stdio/allocator/process/threads/cond/pipe/capability/
W^X/TCC contracts, native fd/PTY/lifecycle/stress, setup/themes/SVG/icons,
полная on-device пересборка Lua, interactive/bytecode и Lua Desktop последним.
Desktop применил Lua policy и committed geometry для пяти окон.
Предыдущие частичные 33+4 результаты выше — история диагностики; теперь есть
один зелёный составной прогон. Disk power-cycle, GUI RAM matrix, Lua failures,
libc-test subset и core/profile gates проверены отдельно с их полными
контрактами. Network/poweroff/reboot/installer-specific suites этим общим
запуском не считаются пройденными автоматически.

Изменения опубликованы тематически: native ABI/process/fd/PTY lifecycle,
GUI/config/terminal/Lua и build/SDK/tests; fork revisions сначала опубликованы
в соответствующих master и только затем закреплены в основном репозитории.
