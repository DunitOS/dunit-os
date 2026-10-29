# Dunit OS / Green Tea Kernel — техническое ревью и roadmap

> Восстановлено из `4e97ddad6aa82ca02fef8d5d4f95c613182c2bc3` (12 октября 2026), исходные статусы сверены с `4ac6113` 24 октября 2026; ниже добавлены проверенные результаты доведения M6. Структура M7 SDK → M8 network → M9 apps → M10 browser и решения §31 сохранены. `[x]` — выполненный конкретный пункт; `[~]` — частично; `[ ]` — не выполнено. Закрытая задача не означает закрытый milestone: нужны его acceptance gates.

### Источник восстановления и граница подтверждённого

`4e97dda` доступен на GitHub, но не является предком текущего `main`.
В сохранившемся reflog 21 октября в 20:38:13 (+05:00) коммит `3727953`
продолжает **`15aaea3` от 11 октября 19:53**, а не более поздний `4e97dda` (между ними 21 коммит).
Затем `filter-branch` переписывает `3727953` в `8e9d80d`.
Roadmap **до и после** этого rewrite имеет один blob
`d314565113169aa16cb91a756a10714792965842`, совпадающий с текущей старой
версией из `e49387f`. У нужной версии другой blob:
`ffa585dc15b953a2e07e2b618f5b36189bda16dd`.
Следовательно, rewrite сам не заменил файл: линия разработки уже была
продолжена с более ранней точки, без поздних изменений 11–12 октября.
Кто и какой удалённой операцией переключил опубликованный `main`, локальный
reflog не доказывает; отдельного коммита отката roadmap в нынешней истории нет.

Важно: потерянная линия содержит не только документ, но и более поздние
реализации `fcntl/poll/ioctl`, file-backed mmap, barrier и spawn file-actions.
Восстановление этого Markdown **не возвращает их код**. В текущем
`abi/syscalls.abi` номера заканчиваются на 84; `posix_spawn.c` игнорирует
`fa/attr`. Эти пункты ниже не помечены готовыми. Исторические выводы и логи
§3 оставлены как история, не как новый прогон всей milestone-матрицы.

Подтверждения обновления: исходники и действующие marker contracts; свежий
GUI/native SVG прогон на `4ac6113` (все пять тем, 7 shell / 6 file icons,
последовательные пресеты), исходный GUI-smoke и пять разрешений до 3072×1920.
M7–M10 не закрываются наличием прототипов или одних только ассетов.


### Сверка статусов на 24 октября 2026

| Раздел | Что действительно есть / что блокирует завершение |
|---|---|
| M0–M3 | ABI manifests, preemption/threads/TLS/futex/handles, protocol tests и userspace compositor; исходный GUI-smoke прошёл на `4ac6113`. |
| M4 | Рабочие desktop/apps, setup/Settings, TOML/Lua, темы/SVG/Chameleon, pointer/mode switch до 3K; session/config persistence через reboot остаётся открытым. |
| M5 | DunitFS v2/fsck/installer/init/profiles — рабочая база; MemFS root и незавершённые user profiles не позволяют закрыть installed-system milestone. |
| M6 | Static C, pthread/condvar, dup и on-device TinyCC подтверждены; native pipe/fd actions/poll/ioctl, boot-UTC clock, libc-test subset и 1000 join + 1000 detached проверены; secure entropy, broader POSIX и release gates открыты. |
| M7 | Выбран runtime `no_std + alloc` поверх `libdunit`; живой control-канал объединён в `gui_protocol_v1::control` и проверен в QEMU. Добавлены `WaitSetWait` и начальный SDK event loop; cargo dunit/Component, полное widget поведение, clipboard/DnD и C GUI SDK ещё не готовы. |
| M8 | Packet networking ещё нет; reboot/poweroff уже есть, остальные platform services открыты. |
| M9–M10 | Полировка имеющихся GUI начата; rock, SDK-приложения и браузер не закрыты. |

Историческая проверка базы `4ac6113` через `tools/qemu_test.py` использовала неизменённые contracts
`m6_musl_hello`, `m6_musl_thread`, `m6_cond`, `m6_pipe`, `m6_dup`,
`m6_tcc_full`, `lua_system`, `svg` (17 команд, включая interactive Lua
и полную пересборку Lua исходников через on-device TCC).
Все required markers, **кроме двух pipe success markers**, получены.
Итог общего audit-прогона — **FAIL**, единственный forbidden marker:
`[PIPE] FAIL pipe`; его нельзя выдавать за зелёный M6.
Это зафиксированная неисправность той базы. В M6-доведении ниже она исправлена и перепроверена; исторический FAIL не удаляется. Отдельный свежий production GUI-прогон
`tools/svg_ui_markers.json` **прошёл**: смена пресетов, 7 shell icons и 6 Files icons, без запрещённых markers.
ABI generator check: **85 syscalls + 25 errno + 7 rights**, все outputs
согласованы с kernel. Наличие файлов тестов без результата не использовалось
как новое доказательство прохождения полной storage/release матрицы.

## 1. Название и цель проекта

**Dunit OS** — самостоятельная x86_64 ОС с ядром **Green Tea Kernel**, собственным ABI, userspace, DunitFS и будущим desktop-стеком. POSIX и musl рассматриваются как слой переносимости исходного кода, а не как цель Linux-совместимости.

Целевая система:

```text
Limine (BIOS/UEFI)
  -> Green Tea Kernel
       memory / VM / scheduler / processes / IPC / VFS / drivers / security
       framebuffer + raw input primitives (не desktop widgets)
  -> init / service manager
       -> GUI Server + compositor
       -> Dunit DWM
       -> terminal, shell and applications
       -> Dunit UI Runtime
  -> DunitFS system and user volumes
  -> Dunit ABI + libdunit + Dunit musl fork
```

## 2. Краткое резюме текущего состояния

Dunit уже больше, чем boot-screen: она загружается через Limine, имеет ring-3 ELF-процессы с отдельными address spaces, generated syscall ABI (`abi/*.abi`), VFS/MemFS, файловые дескрипторы, preemptive UP-планировщик с schedulable user threads, блокирующий IPC, shared-memory handles с rights, userspace-компоситор `gui_server`, DunitFS на `/persist`, BIOS/UEFI disk image, статический musl-порт (`x86_64-dunit`) и on-device компилятор C (TinyCC), который компилирует+линкует+запускает программы на самом ядре.

Однако это ещё не production desktop OS:

- UP round-robin, preemption, schedulable user threads, timer/IPC wait queues, x86_64 static TLS и futex (`FutexWait/Wake`) работают; но **единого wait-set** (ожидание набора из IPC+таймеров+файлов+пайпов одним вызовом) ещё нет — каждый класс объектов ждётся отдельным вызовом;
- нет dynamic TLS/DTV, signals, полноценного process-image `exec`/`fork`, demand paging/COW и file-backed mmap; syscall Mmap принимает только anonymous/private mapping (`kernel/src/syscall/mod.rs`).
- установленная система по-прежнему использует MemFS как `/`; DunitFS автоматически монтируется только в `/persist`;
- DunitFS v2 имеет bitmap/extents, generations/metadata COW, timestamps/mode, rename/unlink/fsync и fsck/degraded mount; таблица метаданных всё ещё ограничена 64 узлами (`kernel/src/fs/dunitfs.rs`). Полноценный installed root и профили пользователя не завершены.
- сеть — discovery E1000 без packet I/O; audio и ACPI power отсутствуют; USB не дошёл до enumeration/class-driver path;
- SMP не реализован (ядро одно-процессорное; «SMP-ready» комментарии в коде не отражают реальности).

Главный вывод: kernel prerequisites для GUI (preemption, блокирующий IPC, shared-memory handles, устойчивый service/process lifecycle) **выполнены** — userspace-GUI уже живой. Дальнейший рост идёт через SDK (M7), сетевой сервис (M8) и приложения/браузер (M9–M10), а не через ядро.

## 3. Методика и результаты проверки

Единственная точка запуска и тестирования: `tools/qemu_test.py`.

Проверено автоматически:

- Terminal ISO: boot smoke + `runtime_stress`, `ipc_parent`, `file_api_test` — успешно;
- GUI ISO 1600×900: дошёл до `[GUI] two-pass blur cache ready`, framebuffer screenshot получен — успешно;
- установленный BIOS disk image: загрузка с HDD, AHCI `sda`, DunitFS auto-mount — успешно;
- установленный UEFI disk image под OVMF: загрузка с ESP, AHCI `sda`, DunitFS auto-mount — успешно;
- persistence: `touch /persist/probe.txt`, полный stop/start QEMU, затем `cat /persist/probe.txt` — файл найден после reboot;
- payload modules закономерно отсутствуют при disk boot: installer payload нужен Live ISO, не установленной системе.

Релевантный tail Terminal Mode:

```text
[STDOUT] runtime_stress: fault OK
[STDOUT] runtime_stress: OK
[PROCESS-RUN] exited pid=4 code=0
[EXEC] /app/runtime_stress returned code=0
[PROCESS-RUN] VFS handles clean
[STDOUT] ipc_parent: OK
[EXEC] /app/ipc_parent returned code=0
[STDOUT] file_api_test: OK
```

Релевантный tail GUI Mode:

```text
[GUI] Starting built-in desktop loop
[GUI] renderer init start
[GUI] back buffer enabled
[GUI] dirty cursor redraw enabled
[GUI] wallpaper loaded from VFS
[GUI] rebuilding two-pass blur cache
[GUI] two-pass blur cache ready
```

Релевантный tail installed BIOS/UEFI boot:

```text
[AHCI] SATA disk port=0 sectors=524288
[AHCI] controllers found=1 initialized=1 disks=1
[GPT-POLICY] matched kind=system
[DUNITFS] auto-mounted sda kind=system at /persist
[ OK ] Dunit OS (Green Tea) ready
[TERM-007] Starting main loop
```

Persistence после reboot:

```text
[GPT-POLICY] matched kind=system
[DUNITFS] auto-mounted sda kind=system at /persist
root@dunit:~# cat /persist/probe.txt

root@dunit:~#
```

Проверен именно жизненный цикл пустого файла. Автотест содержимого, crash-consistency и power-loss recovery ещё отсутствует и обязателен ниже.

## 4. Техническое ревью по подсистемам

### 4.1 Boot, Limine, BIOS, UEFI и режимы

- **Реализовано:** Limine handoff, HHDM, memory map, framebuffer и modules в `hal/src/boot_main.c:166`; выбор Terminal Mode по `mode=terminal` в `hal/src/boot_main.c:205`; меню в `limine.conf`; deterministic configs `limine_test_terminal.conf` и `limine_test_gui.conf`.
- **Реализовано:** host installer создаёт GPT, FAT32 ESP и DunitFS, ставит Limine BIOS stages (`tools/install_disk.py`). Текущий installer ESP payload — 128 MiB; BIOS/UEFI и AHCI/VirtIO имеют отдельные проверки.
- **Работает:** in-system installer транзакционен (validate→partition→format→copy→verify→boot config→sync с rollback) и принимает writable AHCI и VirtIO target (`kernel/src/storage/installer.rs` `is_supported_target`); проверено на матрице BIOS/UEFI × AHCI/VirtIO.
- **Работает:** `initrd::init()` читает реальный `DUNITRD1`-архив из Limine module memory (`kernel/src/initrd.rs`, zero-copy `&'static` срезы), и `kernel/src/fs/vfs.rs` наполняет корневой MemFS из его записей. Архив собирается `tools/pack_initrd.py`.
- **Работает:** приложения (`/app/*`), ассеты (`/assets/**`) и DWM-конфиг грузятся из initrd, НЕ запекаются в ядро. Единственный оставшийся `include_bytes!` в ядре — boot splash (`kernel/src/lib.rs`). Образ ядра не содержит бинарников приложений и desktop-ассетов.

### 4.2 Память и процессы

- **Рабочая база:** multi-region PMM, HHDM, растущая PMM-backed kernel heap; `AddressSpace` с owned user frames, map/unmap и kernel-half sync (`kernel/src/memory/vmm.rs:143`, `:202`, `:227`).
- **Рабочая база:** ELF parser/loader, W^X flags, user stack и `argc/argv/envp` (`kernel/src/elf/mod.rs:209`, `:391`). Userspace faults завершают процесс, не kernel.
- **Реализовано после исходного среза:** anonymous private `mmap`, `munmap`, `mprotect`, guard pages и shared VM object с физическими страницами и refcount. `[VM-TEST] OK` проверяет aliasing между процессами, page fault на guard page, partial unmap и освобождение frame после teardown.
- **Не реализовано:** file-backed `mmap`, demand paging и COW. В текущем `sys_mmap` допустимы только private anonymous mappings; `musl_mmap` и соответствующих marker contracts из потерянной линии в `main` нет.
- **Не реализовано:** `fork` и process-image `exec`. Dunit-native `spawn`/`posix_spawn` работают с argv/env; file-actions open/close/dup2/chdir поддержаны атомарным native spawn contract, остальные attributes явно отклоняются.
- **Исправлено (M1):** все `static mut` singletons ядра убраны (OnceCell/UnsafeCell-newtype/атомики/IrqSafeSpinLock); на UP это корректно. VFS/DunitFS/PMM под IRQ-free синглтонами (единственный async-источник — таймерный IRQ — их не трогает).

### 4.3 Планировщик, threads, TLS и синхронизация

- **Реализовано:** PID ready queue, saved CPU context, transitions Ready/Running/Blocked/Dead/Reaped, `yield`, parent/child wait/reap.
- **Реализовано после исходного среза:** UP round-robin включён по умолчанию; x87/MMX/SSE сохраняются через FXSAVE/FXRSTOR, PIT предоставляет monotonic clock/deadline abstraction. QEMU smoke проверяет вытеснение и XMM isolation.
- **Реализовано после исходного среза:** schedulable userspace TID, отдельные GPR/FPU/kernel stack, общее process-owned address space и fd table; create/exit/nonblocking join/get_tid. QEMU smoke проверяет общую память, XMM isolation, thread fault и teardown при выходе процесса.
- **Реализовано после исходного среза:** wait queue переводит потоки в Blocked; PIT будит `sleep` и timed IPC event wait, отправка IPC будит ожидающие потоки. GUI apps ждут IPC-события без цикла `yield`; QEMU `[WAIT-TEST] OK` проверяет таймаут, sleep и IPC wakeup.
- **Реализовано после исходного среза:** `FS.base` переключается для каждого TID, syscalls 39–40 задают/читают thread pointer, ELF `PT_TLS` создаёт отдельный initial TLS/TCB для main и новых потоков. `[TLS-TEST] OK` проверяет `.tdata/.tbss` и изоляцию при вытеснении.
- **Реализовано:** futex (`FutexWait/Wake`, 41–42) используется базовыми musl pthread/condvar (`musl_thread`, `musl_cond`). Native barrier из потерянной линии не восстановлен.
- **Реализовано в начале M7:** call-scoped level-triggered `WaitSetWait` (89): IPC+timer+file/pipe/PTY+child exit, безопасная `libdunit::wait_set`, `dunit::event::EventLoop`; edge/persistent registration ещё нет.
- **Не реализовано:** dynamic thread vector для dynamic TLS, priorities, time accounting, SMP.
- **Следствие:** `gui_server` как long-running userspace service и musl pthreads **работают и проверены** (M3/M4, pthread subset); остаётся dynamic TLS/DTV и расширение wait-set до edge/persistent registration.

### 4.4 Syscalls и ABI

- **Реализовано:** generated ABI (`abi/*.abi` → `tools/gen_abi.py`), номера 0–89 (85 зарезервирован): file I/O, anonymous mmap, IPC/WaitEvent, framebuffer/input capabilities, spawn/wait/kill, cwd/stats, seek/fstat/dup/pipe/truncate, clocks, threads/TLS/futex. `fcntl/poll/ioctl`, spawn file-actions и начальный level-triggered wait-set также реализованы; wait-set ещё не имеет edge/persistent registration.
- **Удачно:** user-copy проверяет присутствие и writable/user flags каждой страницы до доступа.
- **Безопасность:** raw framebuffer/input — через capability handles (DISPLAY_MASTER/INPUT_MASTER, exclusive), доступные только владельцу (gui_server); не-владелец получает `EACCES`.
- **ABI-долг (остаток):** edge/persistent registration для wait-set и dynamic TLS/DTV. Остальное из прежнего списка (manifest, handle rights, dup/pipe, seek, realtime clocks, robust errno) **сделано**.

### 4.5 IPC

- **Работает:** bounded byte messages до 256 байт, per-PID queues, sender identity; parent/child round trip (`kernel/src/ipc/mod.rs`).
- **Работает:** блокирующий `WaitEvent` (33, IPC+timer), `Endpoint` handle objects, rights transfer (`HandleTransfer` 54).
- **Работает как GUI buffer protocol:** shared-memory capability path (`HandleCreateShared` 56, `handle_map`, `HandleTransfer`, `FbPresent` 57) — именно это использует живой gui-v1 (M3). Отдаёт handle с rights, не physical address.

### 4.6 VFS, MemFS, DevFS/ProcFS и DunitFS

- **Работает:** path normalization, mounts, open/read/write/close/create/mkdir/remove/truncate/readdir/stat, per-process fd table; MemFS — root (`kernel/src/fs/vfs.rs:487`).
- **Частично:** DevFS/ProcFS существуют, но не являются полноценными независимыми mounts/device interfaces.
- **Работает:** DunitFS v2: bitmap/extents, directories, rename/unlink, timestamps/mode, fsync, generations и metadata COW, fsck/recovery и degraded read-only mount (`kernel/src/fs/dunitfs.rs`; `tools/m5_dunitfs_v2_markers.json`, `tools/m5_fsck_markers.json`).
- **Ограничения DunitFS:** 64 metadata nodes; symlinks/hardlinks и полноценная users/groups/ownership-модель не завершены. Root по-прежнему MemFS из initrd, `/persist` — отдельный writable mount; M5 целиком не закрыт.
- **Исправлено (M5 item 1):** `auto_mount` теперь сверяет GPT type GUID по политике (`kernel/src/storage/policy.rs`) и монтирует только Dunit-owned partition (System/Data); foreign partition отсекается до проверки superblock.

### 4.7 Block storage, GPT и установка

- **Работает в QEMU:** PCI discovery, AHCI DMA read/write/flush, SATA registration; legacy VirtIO block polling; generic block registry; GPT primary/backup read/write with CRC.
- **Работает:** host image path создаёт ESP + DUNIT-SYSTEM (Dunit System type GUID); installed boot и `/persist` подтверждены.
- **Работает:** in-system installer транзакционен (validate→partition→format→copy→verify→boot config→sync с rollback), AHCI+VirtIO, BIOS/UEFI (M5).
- **Ключевой разрыв:** `/app` и ассеты грузятся из initrd (не embedded), а корневая ФС всё ещё MemFS; disk-backed system root (`/` на DunitFS) — отдельная открытая задача. DUNIT-SYSTEM пока data volume + `/persist`.

### 4.8 Terminal Mode и GUI Mode

**Terminal Mode**

- framebuffer console и kernel shell реально работают; history/autocomplete, VFS, diagnostics и foreground ELF exec подтверждены;
- shell и terminal loop находятся в ядре (`kernel/src/lib.rs:750`), поэтому это bootstrap/recovery console, а не конечный userspace shell;
- это наиболее стабильный режим разработки.

**GUI Mode (userspace, M3/M4 — legacy in-kernel GUI удалён)**

- compositor — отдельный userspace ELF `gui_server` (`userspace/system_apps/gui_server/src/main.rs`): software-композитинг в ARGB back-buffer, damage tracking (протокол), прозрачность/blur/скругления/тени/градиенты для декораций, focus/input routing, окна, панель/dock/launcher/tray/quick-settings/notifications/workspaces/switcher, wallpaper, software-курсор. Полностью config-driven (`dwm_settings` + `/system/share/dwm/*.toml`), без hardcoded `match app`;
- протокол `gui_protocol_v1` (`protocols/gui-v1/`, магия `DGUI`): версия/фичи, surface+buffer lifecycle, configure/ack/commit, frame callbacks, shared-buffer через capability-handle (не immediate `DRAW_RECT`); клиенты — независимые ELF-процессы, маршрутизация по kernel-authenticated pid, а не по magic PID 1;
- `gui_terminal` — независимый userspace терминал поверх `dsh` (PTY-slave), не kernel `shell::run_command`;
- **легаси in-kernel GUI (`kernel/src/ui_loop.rs`, `window_manager.rs`) удалён целиком**; раздел §15 ниже — исторический план этого выноса (выполнен).

**Разрывы текущего GUI (вход для M7 SDK):**

- живой input/control-канал `CAP1/INP1/RLD1/NOT1` централизован в `gui_protocol_v1::control`, который используют `gui_server` и 7 клиентов; wire-формат сохранён. Формальный input-роутер DGUI пока используется только в self-тестах; live damage и SDK event loop ещё не подключены;
- живой цикл перерисовывает весь экран каждый тик (damage протокола используется только в self-тесте); нет интерактивного resize-grip, системного DnD и буфера обмена (в протоколе — явный non-goal v1);
- оболочка (панель/dock/launcher) нарисована целиком in-process в `run_desktop_session`; вынос в отдельный клиент — оценивается в M7.

### 4.9 Драйверы и будущая платформа

- framebuffer, PS/2 keyboard/mouse, PCI, AHCI и basic block I/O — рабочая QEMU-база;
- VirtIO block — legacy polling, не modern PCI transport и не interrupt-driven;
- E1000 читает status/MAC, но `packet-io=not-implemented` (`kernel/src/drivers/net.rs`);
- xHCI делает controller/ring/Enable Slot bring-up, но USB enumeration, descriptors, endpoints и class drivers отсутствуют;
- audio отсутствует;
- Reboot/poweroff имеют рабочие команды и отдельные QEMU-тесты; ACPI battery/thermal и suspend/resume остаются незавершёнными.

## 5. Что production-like, частично и prototype

| Уровень | Компоненты | Оценка |
|---|---|---|
| Production-like для hobby/QEMU CI | Limine BIOS/UEFI boot, Terminal boot, PMM/VMM base, ELF smoke, safe user-copy, MemFS/VFS, AHCI/VirtIO disk, generated ABI, initrd loader | Повторяемо работает, но ещё не hardened для hostile workloads; SMP нет |
| Интегрировано (end-to-end) | processes, preemptive scheduler, threads+TLS+futex, syscalls, blocking IPC+handles, DunitFS/persist, GPT/install, UEFI, userspace GUI (gui_server+apps), musl static libc, on-device tcc | Реальные end-to-end пути; часть семантики/recovery неполна |
| Prototype | VirtIO legacy block, xHCI bring-up, network discovery | Нельзя стабилизировать как public API в текущем виде |
| Не реализовано | signals, fork/COW, dynamic loader/TLS, единый wait-set, demand paging, audio, TCP/IP (M8), ACPI power, package manager, SMP | Roadmap ниже (M7 SDK → M8 сеть → M9 → M10) |

## 6. Реализованные компоненты

- [x] BIOS/UEFI Limine boot, Terminal/GUI mode selection and framebuffer handoff.
- [x] PMM/VMM/heap, isolated ring-3 ELF execution and recoverable userspace faults.
- [x] Dunit syscall ABI, VFS/MemFS, per-process FDs, cwd, argv/envp and file APIs.
- [x] Preemptive UP process/thread lifecycle, bounded IPC messages and automated runtime regression.
- [x] PCI, AHCI block I/O, GPT, host/in-system installer foundations and DunitFS `/persist` mount.
- [x] Userspace GUI compositor/DWM и независимые ELF-клиенты; старый in-kernel desktop удалён, kernel сохраняет display/input mechanisms.

## 7. Незавершённые компоненты

- [x] Default-on preemption, userspace threads и blocking timer/IPC wait queues (M1 QEMU smokes).
- [x] Static TLS, native futex и базовый musl pthread subset: create/join, mutex и condvar. Подтверждения: `[TLS-TEST] OK`, `futex_test: OK`, `tools/m6_musl_thread_markers.json`, `tools/m6_cond_markers.json`. Dynamic TLS, cancellation и дополнительные pthread primitives — отдельные gates.
- [x] Safe shared-memory objects and capability-based IPC handles. Shared VM object с guard pages/W^X/teardown (`[VM-TEST] OK`) и per-process handle table с rights (`READ/WRITE/MAP/SIGNAL/TRANSFER/DISPLAY_MASTER`, права только сужаются; `handle_test: OK`) — на M1; активно используются gui-v1 (shared framebuffers + capability-передача буфера компоситору).
- [x] Userspace GUI Server/compositor/DWM and declarative UI Runtime. **Готово:** userspace `gui_server` (software compositor, damage, focus/input routing), config-driven DWM (панель/dock/launcher/workspaces/widgets, quick settings, notifications, Alt/Super switcher, config-driven край панели/трея, protocol resize/maximize, ARGB-прозрачность) и declarative UI Runtime (крейты `runtime/`, `gui_client` рисует DUI+DSS). Остаётся только persistence session-store через ребут (зависит от M5/DunitFS v2 — учтено в M4).
- [~] Installed-system foundation: init/service supervision, first-boot marker/config directory, DunitFS v2/fsck и installer готовы; disk-backed `/`, полноценные user profiles и desktop-config persistence через reboot не готовы.
- [~] Static musl/toolchain: C suite, file APIs, malloc, базовые pthread/TLS, clocks, spawn/wait/kill, dup и on-device TinyCC готовы; Pipe=83 есть в kernel, но libc pipe gate сейчас падает (ENOSYS). Открыты: `posix_spawn` file-actions/attributes (сейчас игнорируются), fcntl/poll/select/ioctl, file-backed mmap, barrier, libc-test, CMake/Meson/Autoconf cross files; signals/dynamic linking и сеть — позже. Не переносить `[x]` из старой ветки без возвращённого кода и теста.
- [ ] Packet networking, audio, complete USB, ACPI/power and package/application platform.

## 8. Удачные решения и главные архитектурные проблемы

Удачные решения:

- Rust `no_std` kernel + компактный C/NASM HAL дают ясную границу hardware entry/ядро;
- Dunit-native `spawn/yield/wait` позволяет развиваться без преждевременного `fork`;
- отдельные address spaces, recoverable userspace faults и safe-copy — правильный фундамент;
- VFS mount abstraction уже позволяет заменить embedded root постепенно;
- общий block registry отделяет DunitFS/GPT от AHCI/VirtIO;
- test-only Limine configs и автоматический QEMU harness обеспечивают воспроизводимость.

Мешавшие решения (в основном устранены — осталось отмечено):

- ~~4458-строчный `ui_loop.rs` объединял shell/protocol/compositor/DWM/widgets в kernel~~ — удалён; GUI вынесен в userspace `gui_server` (но тот теперь сам монолит 5143 строки — цель рефакторинга M7.9);
- ~~raw pixel/input syscalls обходили ownership~~ — framebuffer/input за capability-handle (DISPLAY_MASTER/INPUT_MASTER), не-владелец → EACCES;
- ~~magic PID 1 вместо registry~~ — маршрутизация по kernel-authenticated pid + `Endpoint`-handle; `init` — service registry по манифестам;
- ~~embedded apps/assets~~ — грузятся из initrd, kernel rebuild развязан с userspace;
- ~~cooperative execution~~ — preemptive UP-планировщик + блокирующий IPC/futex;
- global `static mut` убраны (OnceCell/атомики/IrqSafeSpinLock) — но single-CPU assumptions остаются: **SMP не реализован**;
- **осталось:** DunitFS data model не годится как системный root без versioning/recovery (disk-backed root — открытая задача).

## 9. Целевая архитектура Green Tea Kernel

Kernel оставляет только mechanisms:

- PMM/VM: address spaces, `map/unmap/protect`, shared VM objects, W^X, guard pages;
- processes/threads: preemptive scheduler, wait queues, clocks, TLS base, credentials;
- object/handle IPC: endpoints, events, shared buffers, rights transfer;
- VFS and block primitives;
- framebuffer/display handle и input event devices;
- capability checks: только GUI Server получает display/input master rights;
- crash reporting, logs и service supervision hooks.

Kernel не должен содержать dock, panel, launcher, decorations, widgets, themes, layout parser или application-specific drawing.

## 10. Архитектура GUI Server, compositor и протокола

Процессная модель:

```text
init (PID 1)
  |- gui-server       owns display/input master handles
  |- dunit-dwm        privileged shell client
  |- terminal         ordinary GUI client + PTY/session
  |- notificationd    optional service after v1
  `- applications     untrusted clients
```

GUI protocol v1 должен быть binary, versioned и transport-neutral:

- `HELLO(version, features)` / `WELCOME(client_id, formats)`;
- `CREATE_SURFACE(role, width, height, format)` -> surface handle;
- `ATTACH_BUFFER(surface, shm_handle, offset, stride, damage[])`;
- `COMMIT(surface, serial)`; server releases buffer asynchronously;
- `CONFIGURE(surface, size, scale, state, serial)` / `ACK_CONFIGURE`;
- `POINTER_*`, `KEY_*`, `TEXT_INPUT`, focus enter/leave;
- `SET_TITLE`, `SET_APP_ID`, `REQUEST_CLOSE`;
- privileged DWM operations separated from ordinary client operations.

Surface format v1: premultiplied XRGB8888/ARGB8888, explicit width/height/stride, bounded damage rectangles, double/triple buffering. Client never receives the physical framebuffer address.

Compositor:

- z-order scene graph, clipping, occlusion and damage tracking;
- focus policy delegated to DWM policy API, enforcement remains server-side;
- input hit-testing against committed scene;
- frame callbacks rather than busy redraw;
- deterministic software compositor first; GPU acceleration later behind backend trait.

Failure policy: malformed message disconnects client, stale handle returns protocol error, crashed app destroys only its surfaces, crashed DWM does not crash GUI Server, crashed GUI Server is restarted by init or falls back to Terminal Mode.

## 11. Dunit UI Runtime и config-driven desktop

Рекомендуемое разделение форматов:

| Назначение | Формат | Причина |
|---|---|---|
| Settings, app metadata, panel/dock composition | TOML subset | Читаем человеком, простая schema/version, подходит persistence |
| UI component tree | DUI — небольшой declarative markup | TOML/JSON неудобны для глубокого дерева, а HTML слишком велик |
| Appearance | DSS — CSS-подобный ограниченный stylesheet | selectors, states, variables и themes без полной сложности CSS |
| Motion | declarative blocks в DSS/DUI | duration/easing/properties; никакого arbitrary script в v1 |

Не использовать полный CSS/HTML/JS engine на раннем этапе. JSON оставить для tooling/IPC diagnostics, не как основной hand-edited config.

UI Runtime:

- retained component tree с stable IDs;
- `Row`, `Column`, `Stack`, `Grid`, `Scroll`, constraints, min/max/preferred size;
- widgets: text, icon, button, input, list, menu, slider, progress, notification;
- event capture/bubble, focus navigation, keyboard actions;
- style cascade: defaults -> theme -> component class -> state;
- animation clock и property interpolation;
- parse/validate off-screen, atomic tree swap; на ошибке оставить last-known-good и показать diagnostic;
- filesystem watch через будущий event API; до него explicit reload action;
- runtime не знает о dock/launcher: DWM собирает их как обычные UI trees.

Пример файлов:

```text
/system/share/dunit-ui/defaults.dss
/system/share/dwm/desktop.dui
/system/share/dwm/default.toml
/users/<uid>/config/dwm/config.toml
/users/<uid>/config/dwm/theme.dss
/users/<uid>/config/dwm/desktop.dui
```

## 12. Архитектура Dunit DWM

Dunit DWM — отдельный policy shell поверх GUI Server:

- desktop/workspaces, panel, dock, launcher, quick settings, notifications, window switcher;
- app registry из manifests, а не enum;
- placement/focus/workspace rules из TOML;
- wallpaper loader и scaling в userspace;
- brightness UI вызывает power/display service, не затемняет software framebuffer как финальное решение;
- terminal, file manager, calculator и monitor — отдельные clients;
- DWM не рисует client contents и не владеет чужими buffers.

## 13. Что нужно улучшить в kernel до DWM

Минимальный gate для DWM: M1 должен дать preemption, threads, blocking events, безопасные shared VM objects и rights-bearing handles. `gui-server` должен запускаться и перезапускаться как service, а не вызываться из вечного kernel loop. Raw input/framebuffer выдаются только ему; приложения получают protocol endpoints. До этого допустимы host-side protocol/layout tests и legacy GUI maintenance, но не фиксация нового desktop ABI.

## 14. Milestones и task cards

### M0 — зафиксировать контракты и убрать ложный статус (must-have)

- [x] Написать `docs/architecture/current-state.md` и Dunit ABI contract table (`abi/README.md`, текущая версия v0).
- [x] Исправить boot logs initrd/shared-memory/context-switch, чтобы они отражали измеренное состояние.
- [x] Сделать compiler warnings и QEMU markers частью CI budget.

**Цель/причина:** создать честный baseline; без него нельзя отличить integration от файла-заготовки. **Подсистемы:** boot, docs, tests. **Зависимости:** нет. **Результат:** versioned snapshot. **Готовность:** каждый WORKING claim имеет QEMU test. **Тесты:** Terminal/GUI/BIOS/UEFI matrix. **Риск:** документация снова устареет; снижать executable status report.

### M1 — kernel runtime prerequisites (must-have, перед GUI migration)

- [x] Доказать, что preemptive round-robin на PIT реально вытесняет CPU-bound child без `yield` (smoke-хук `[PREEMPT-TEST] OK`).
- [x] Включить round-robin по умолчанию, сохранять FPU/SSE-состояние и добавить abstraction clocksource/timer. Проверено QEMU boot smoke: CPU-bound parent/child с разными XMM значениями, `runtime_stress`, `ipc_parent`.
- [x] Сделать schedulable threads: TID, per-thread context/kernel stack/FPU state, process-owned address space. QEMU `[THREAD-TEST] OK`: два потока делят память/PID, сохраняют разные XMM значения, join возвращает статусы, thread fault изолирован, unjoined поток удаляется при выходе владельца.
- [x] Добавить wait queues и blocking sleep/IPC event wait; убрать polling GUI apps. `[WAIT-TEST] OK` проверяет timeout, timer wake и IPC wake; общий handle/event readiness остаётся отдельной задачей.
- [x] Реализовать `munmap`, `mprotect`, shared VM object, guard pages и correct teardown. QEMU `[VM-TEST] OK`: partial unmap, W^X, guard fault, shared aliasing и возврат frame в PMM после закрытия/выхода peer.
- [x] Добавить TLS ABI: set/get thread pointer (`FS.base` на x86_64), initial `PT_TLS` image. `[TLS-TEST] OK`: Variant II TCB, `.tdata/.tbss`, отдельный TLS main/threads и сохранение при вытеснении.
- [x] Добавить futex-like Dunit primitive `wait_on_word/wake` без копирования Linux ABI. Dunit-native: ключ `(owner pid, user vaddr)`, `FutexWait=41`/`FutexWake=42`, compare-and-park атомарен под IRQ guard + WAIT_QUEUE (без lost wakeup), `read_user_u32` проверяет PRESENT|USER. QEMU `futex_test: OK`: EAGAIN на несовпадении, timeout, wake/join worker'а.
- [x] Перевести глобальные mutable singletons на locks/owned services. Все `static mut` ядра убраны: write-once синглтоны → `OnceCell`/`UnsafeCell`-newtype с `unsafe impl Sync`; состояние, разделяемое с IRQ (буфер клавиатуры) → `IrqSafeSpinLock`; данные под уже существующими spinlock'ами (MMIO, PCI, registry, block, mouse-пакет, ring событий мыши) свёрнуты в одну структуру за `UnsafeCell` под тем же локом; простые скаляры-флаги (параметры фреймбуфера, флаги init) → атомики; кооперативные синглтоны, раздающие `&'static mut` (VMM/PMM/VFS/process table/IPC/WM/initrd/kthreads/dunitfs), → `UnsafeCell`-newtype с сохранением семантики UP. Остались только 2 символа `extern "C"` от HAL (`boot_mem_regions/boot_mem_region_count`), читаемые один раз через `read_volatile`. Регрессии зелёные: boot до терминала, `[TLS-TEST] OK`, `futex_test: OK`.
- [x] Ввести handle table с rights (`READ/WRITE/MAP/SIGNAL/TRANSFER/DISPLAY_MASTER`). Per-process `HandleTable` (kernel/src/handle.rs): непрозрачный `Handle` -> объект ядра + битовая маска прав. Объекты: `Memory` (READ/WRITE/MAP), `Endpoint` (SIGNAL), `Display` (DISPLAY_MASTER, эксклюзивно на систему). Инвариант capability: права можно лишь сужать через `dup`/`transfer`, расширение отклоняется. Syscalls 43–54 (create_memory/create_endpoint/read/write/map/dup/rights/close/signal/take_signals/display_acquire/transfer) + обёртки в libdunit. Teardown освобождает дисплей и очищает таблицу при выходе процесса. QEMU `handle_test: OK`: все 6 прав, READ/WRITE/MAP по объекту памяти, SIGNAL через endpoint + приём, эксклюзивный DISPLAY_MASTER, TRANSFER (перенос + отказ без права), сужение прав и невалидность хэндла после close/transfer.

**Цель/причина:** GUI services и musl threads требуют concurrency, blocking и shared buffers. **Подсистемы:** process, scheduler, interrupts, VMM, IPC, HAL. **Зависимости:** M0. **Результат:** несколько независимых long-running processes/threads. **Готовность:** CPU-bound child preempted без `yield`; blocked client не расходует CPU; kill освобождает mappings/handles. **Тесты:** starvation, preemption stress, thread TLS isolation, mmap/unmap/protect, wait/wake race, fault injection. **Риски:** IRQ/lock deadlocks, FPU leakage, lost wakeups; сначала UP, затем SMP.

### M2 — GUI protocol + headless reference server (можно параллельно с M1)

- [x] Специфицировать protocol v1, state machines, object lifetimes, limits и errors. Контракт v1.0: [protocols/gui-v1/README.md](protocols/gui-v1/README.md) — wire layout, negotiation, surface/buffer/configure/focus lifecycle, quotas, errors и нормативные сценарии. Это нормативная спецификация; decoder, headless tests и kernel binding уже реализованы следующими пунктами M2/M3. Заморозка public release ABI — отдельный gate.
- [x] Реализовать host/headless protocol tests без framebuffer. Крейт [protocols/gui-v1](protocols/gui-v1) (`no_std + alloc`, ноль внешних зависимостей): реальный wire-кодек (`src/wire.rs`) и headless reference server (`src/server.rs`) с порядком валидации §10 (framing/attachments → version → serial → opcode direction/policy → connection state → object namespace → enum/geometry/quotas). 37 host-тестов `cargo test`: framing/malformed rejection, negotiation/HELLO timeout/serial monotonicity, object namespace (fresh/stale/reused/cross-connection/type), import bounds/limits и детерминированный byte-identical replay. Поведенческие state machines (surface/buffer/configure/commit/focus/input events) и fuzz/property остаются пунктом 4 ниже.
- [x] Перенести полезные pure structures/tests из `userspace/display_server`, не его текущий `egui` binary lifecycle. Модель [`protocols/gui-v1/src/wm.rs`](protocols/gui-v1/src/wm.rs): чистый, детерминированный window/focus/composition-слой (window stack, focus policy с авто-фокусом и переназначением при destroy, top-most hit-testing, click-drag, CPU-композитор в ARGB `Vec<u32>`) — `egui`-lifecycle (`Context`/`RawInput`, scancode→egui-key) отброшен. Property-тесты display_server перенесены в [`tests/wm.rs`](protocols/gui-v1/tests/wm.rs) как детерминированные развёртки (proptest недоступен offline): creation/drag/focus/compositing/cursor + hit-test стекинга и скрытых окон. Общий host-набор крейта — 45 тестов, зелёный.
- [x] Реализовать surface/buffer/focus/input models и fuzz/property tests. В [protocols/gui-v1/src/server.rs](protocols/gui-v1/src/server.rs) добавлены поведенческие state machines: surface configure/ack/commit lifecycle с configure/commit токенами, buffer ownership (`AVAILABLE→PENDING→BUSY` + `BUFFER_RELEASE`), frame callbacks завершаемые на composition tick, и single-seat focus/input роутер (pointer/keyboard enter/leave, motion/button/axis, key/text, implicit grab, подавление release неудерживаемых keys, held-key overflow). Policy-решения (focus assignment, reconfigure) входят как verified external events (§9). Новый [tests/behavior.rs](protocols/gui-v1/tests/behavior.rs) детерминированно покрывает нормативные сценарии §11.1–4, §11.9, §11.10 плюс seeded byte/event fuzz (no panic, per-connection serial monotonicity); [tests/limits.rs](protocols/gui-v1/tests/limits.rs) закрывает оставшиеся edge-сценарии §11.7/§11.8/§11.12 — damage-rect boundary/max/max+1, `damage_count`/array mismatch → MALFORMED, backing end==cap/overflow, surface dims min/max/max+1, per-connection backing budget, fixed little-endian/no-padding wire round-trip и no-partial-state + retry под новым serial. Всего 61 host-тест `cargo test` зелёный; QEMU boot regression без изменений. Дублирующий import memory-object (§7) отложен до M3 kernel binding (transport shim не несёт memory-object identity), помечено в коде.

**Цель/причина:** отделить контракт от текущего kernel UI. **Подсистемы:** новый `protocol/`, display-server tests. **Зависимости:** финальная интеграция зависит от M1, проектирование нет. **Результат:** стабильная protocol crate/spec. **Готовность:** invalid clients не влияют на server; protocol replay детерминирован. **Тесты:** malformed length, stale IDs, buffer bounds, focus ordering. **Риски:** заморозить слишком широкий API; v1 оставить минимальным.

### M3 — userspace GUI Server и compositor (must-have)

- [x] Добавить display/input master handles и shared-buffer syscalls.
- [x] Собрать `gui-server` как обычный ELF из Makefile/image manifest.
- [x] Реализовать software compositor, damage, frame callbacks, focus/input routing.
- [x] Запустить два untrusted client processes одновременно. `gui_server` спавнит два неприв. `gui_client` ELF, выдаёт каждому 8-байтовый handshake `[our_pid][client_id]`; клиенты рендерят в свой kernel shared buffer, передают его capability (read-only) компоситору и гонят gui-v1 lifecycle по IPC с 4-байтовым client-id конвертом (IPC не сообщает sender pid). Один event-loop роутит сообщения по client-id к своим `Server`-соединениям, релеит ответы, и на каждом `composite()` матчит FRAME_DONE к ConnId, блитя буфер клиента в свой слот. Оба презентят (`gui_client: surface presented OK` ×2 → `gui_server: served two untrusted clients OK`), exit 0, VFS handles clean.
- [x] Старый kernel GUI удалён. Ранее был под `legacy_gui` feature-гейтом до достижения parity; после parity `gui`/`ui_loop`/`window_manager`, сама фича `legacy_gui`, протокол GUI1 в `libdunit` и 5 legacy-клиентов (`gui_ping`/`gui_terminal_stub`/`gui_calculator`/`gui_stats`/`gui_file_manager`) удалены полностью (2026-09-27). Загрузка по `limine_dwm.conf` автозапускает `/app/gui_server` как десктоп через `run_foreground_exec`; в ядре остался только механизм (framebuffer, display/input master, shared buffers, IPC, handles, drivers), recovery — через terminal-режим (`mode=terminal`).

**Цель/причина:** удалить desktop policy из kernel. **Подсистемы:** kernel display/input/IPC, userspace gui-server, build. **Зависимости:** M1, M2. **Результат:** kernel предоставляет только mechanisms. **Готовность:** `kernel/src/ui_loop.rs` не участвует в normal GUI boot; server crash не рушит kernel. **Тесты:** overlapping surfaces, resize storms, app crash, buffer reuse, input isolation, 10-minute idle/load. **Риски:** copies/performance; сначала correctness и bounded buffers.

### M4 — UI Runtime и Dunit DWM parity (must-have)

- [x] Реализовать DUI parser/tree, layout, base widgets, DSS theme и motion engine. Крейты `runtime/` (no_std+alloc, `#![forbid(unsafe_code)]`): `dunit-text` (TTF-растеризатор), `dunit-ui` (DUI parser + retained tree + two-pass content-measured layout), `dunit-style` (DSS каскад с CSS-specificity, `$`-переменные, compound-селекторы, motion), `dunit-widgets` (Widget из тегов, intrinsic size, event-модель), `dunit-render` (CPU-художник Surface + paint). Подключены к боту через `gui_client`, который красит реальное DUI+DSS-окно через весь runtime.
- [x] Реализовать schema-versioned TOML settings и last-known-good reload. **Готово:** last-known-good reload — при отсутствии/повреждении конфига крейт `dwm_settings` откатывается на встроенный baseline (Green Tea), а живой reload через RELOAD_MAGIC валидирует весь конфиг перед применением (повреждённый config не рушит десктоп); явное schema-версионирование — `[meta] schema_version` парсится через тот же путь, что и остальные таблицы, `dwm_settings::version_supported()` (1..=1) принимает текущую версию и отклоняет будущую (документ новее рабочего билда откатывается на last-known-good baseline, а не мис-парсится). Смоук печатает `config schema_version=1 supported=1` + `schema guard ok (rejects future)`.
- [x] Собрать DWM: desktop, panel, dock, launcher, quick settings, notifications, workspaces, Alt/Super switcher. **Готово:** интерактивный userspace-компоситор `gui_server` (полноэкранный back buffer, mouse через syscall 60, draggable title bars, focus/raise/drag/close, роутинг ввода в фокусное окно); DWM-панель + dock (config-driven pin-list, task-switcher, свёртывание/восстановление окон); launcher спавнит произвольные приложения из реестра `[application.*]` (прежний хардкод двух `gui_client` убран); workspaces (переключение по пипсам + crossfade, count из конфига); config-driven край панели и трея (top/bottom/left/right); слоистый конфиг (per-app `apps/*.toml` + per-widget `widgets/*.toml`); protocol resize/maximize (Configure/AckConfigure); пер-пиксельная ARGB-прозрачность; anti-pixelation иконок Breeze; удаление виджетов через конфиг; чистая безоконная загрузка в интерактивный десктоп; Alt/Super window switcher (`[shortcuts] switcher`/`switch_mod`, Tab-инвариант, реально поднимает другое окно); quick-settings applet на панели (флайаут переключает живые `fx.blur`/`wg.enabled` — единый source of truth, не теневая копия; гейт `[quicksettings] enabled`); desktop notifications (bounded toast-очередь, policy из `[notifications] timeout_ms`/`corner`, авто-скрытие по таймауту). Смоук печатает `switcher cycle … ok`, `quicksettings applet enabled=1 … ok`, `notify posted … ok` и `notify expired`.
- [x] Перенести terminal, calculator, file manager, stats на client API. `gui_terminal`, `gui_calc`, `gui_files`, `gui_stat` — независимые userspace-ELF, рисующие через gui-v1 client API (CreateSurface/ImportBuffer/Commit по IPC + shared buffers), зарегистрированы в реестре `[application.*]`; in-kernel версии удалены на M3.
- [x] Перенести GUI terminal command execution в userspace shell/session; добавить PTY-like endpoint. PTY-endpoint в ядре (`kernel/src/pty/`, `SYSCALL_PTY_SPAWN=62`, ring master↔slave); `gui_terminal` спавнит userspace-шелл `dsh` как pty-slave (`libdunit::pty_spawn_env`) и мостит ввод/вывод — исполнение команд полностью в userspace.
- [x] Терминальный `setup`: интерактивные настройки и команды show/get/set/import/check, общий checked TOML overlay с GUI Settings (`userspace/system_apps/setup`, `tools/setup_markers.json`).
- [x] Нативные Lua/luac как системные бинарники; Lua tiling policy запускается отдельным процессом с deadline, проверкой результата и сохранением рабочей геометрии при ошибке. TOML остаётся источником базовых настроек (`tools/lua_system_markers.json`, `tools/lua_desktop_markers.json`). Lua-скрипты интерпретирует Lua VM; TinyCC компилирует C, это разные инструменты.
- [x] Общий static SVG/bitmap loader и конфигурируемые иконки: пять полных Chameleon ZIP, оригинальные пути, index.theme size/scale, aliases/fallback в TOML; dock/launcher/switcher/Files используют один `dunit_image::IconTheme` (`tools/svg_markers.json`, `tools/svg_ui_markers.json`). Полный браузерный SVG standard не заявляется.
- [x] Исправить pointer geometry/границы экрана и held-button state при пересоздании сессии; проверить Settings mode switch до 3072×1920 и обратно с точными framebuffer captures (`tools/pointer_markers.json`, `tools/display_3k_markers.json`). Pending client notifications теперь переживают mode change; исходный GUI-smoke проходит.
- [~] Сохранить/восстановить window/session settings в user config. **Готово:** `gui_server` ведёт session-store (геометрия каждого окна по app id), путь save→persist→reload проверен end-to-end (`[session] restore` + смоук `session roundtrip ok (wrote=1 saved=1 reloaded=1)`, `session restore=1 entries=0` на чистой загрузке). **Осталось:** persistence через ребут — сейчас store RAM-only (переживает reload/reconnect внутри одной загрузки); span reboots появится, когда M5/DunitFS v2 даст persistent user-config partition (зависимость `writable installed user config`).

**Цель/причина:** полный отказ от hardcoded desktop. **Подсистемы:** ui-runtime, DWM, apps, VFS. **Зависимости:** M3 и writable installed user config. **Результат:** внешний вид меняется без kernel rebuild. **Готовность:** все перечисленные функции parity; испорченный config не мешает login. **Тесты:** golden layout at 1024×768/1600×900/HiDPI, keyboard-only navigation, reload, malformed config, animation timing. **Риски:** scope explosion; accessibility/text shaping оставить отдельными incremental milestones, но API учесть.

### M5 — normal installed system и persistence (must-have; storage foundation можно параллельно M1-M3)

- [x] Определить GPT policy: ESP + Dunit System + Dunit Data; для v1 допустимы ESP + единый DunitFS root, но `/system` должен быть read-mostly.
- [~] Убрать `include_bytes!` applications/assets; загрузить init и app manifests с disk root. **Готово:** applications/assets вынесены в настоящий `DUNITRD1`, есть init/service manifests. **Осталось:** загрузка нормального system root с диска вместо MemFS/initrd; initrd сам по себе не закрывает этот пункт.
- [~] Реализовать real init/service manager и first-boot provisioning. **Готово:** service manifests, supervision/restart budget и `/persist/.provisioned` (`userspace/system_apps/init`). **Осталось:** полное provisioning installed user/system profiles поверх disk root.
- [~] Расширить DunitFS: bitmap/extents, directories, rename/unlink, timestamps/mode, fsync, generations и metadata COW реализованы (`kernel/src/fs/dunitfs.rs`, m5 v2/fsck contracts). Полноценная users/groups/permissions-модель не завершена; лимит 64 metadata nodes сохраняется, 1000-file acceptance ниже не закрыт.
- [x] Добавить `fsck.dunit`, read-only degraded mount и recovery report.
- [x] Сделать installer transactional: validate -> partition -> format -> copy -> verify -> boot config -> sync.
- [x] Поддержать AHCI и VirtIO target; отдельно тестировать BIOS и UEFI.
- [~] Разделить Live и installed manifests. **Готово:** отдельные profiles/manifests и проверки `tools/m8_live_profile_markers.json`, `tools/m8_installed_profile_markers.json`. **Осталось:** installed profile с настоящим persistent root вместо MemFS; названия старых тестов `m8_*` не меняют новую нумерацию milestone M5.

**Цель/причина:** `/persist` рядом с embedded root не является законченной установкой. **Подсистемы:** installer, GPT, DunitFS, VFS, boot, init, build images. **Зависимости:** базовая часть параллельна GUI; DWM config persistence зависит от неё. **Результат:** system/users/apps обновляются с диска. **Готовность:** create/write/fsync/reboot/read; interrupted metadata update восстанавливается; kernel не содержит app binaries. **Тесты:** two-reboot content hash, 1000-file/large-file tests, disk-full, corrupted CRC, power-cut points, BIOS/UEFI × AHCI/VirtIO. **Риски:** filesystem corruption; нельзя использовать DunitFS v1 как единственную копию важных данных.

Рекомендуемая иерархия:

```text
/system/bin        trusted system programs
/system/lib        Dunit runtime, libc, shared libraries later
/system/share      themes, DUI/DSS, icons, defaults
/system/services   service manifests
/apps/<id>         application bundles/manifests
/users/<uid>/home  documents
/users/<uid>/config
/users/<uid>/data
/var/log           bounded logs
/var/lib           service state
/run               volatile runtime state
/devices           device namespace
/proc              diagnostics
```

### M6 — Dunit musl fork, static-first (should-have после M1/M5)

Основной кандидат фиксируется: **полный fork musl**. relibc/newlib допустимы только как сравнительные fallback, если ограниченный spike обнаружит фундаментальный блокер. musl проектировалась поверх Linux syscall layer, поэтому Dunit выполняет **порт на новую ОС**, а не только добавляет новую CPU-архитектуру. Готовый Linux `libc.a` и простая замена syscall numbers непригодны: отличаются контракты процессов, файлов, VM, threads, signals, структуры данных и ошибки.

#### M6.1. Что является портом, а что не является Linux-совместимостью

Целевая цепочка:

```text
C/POSIX source API
        |
        v
Dunit musl fork: libc semantics + Dunit OS adaptation layer
        |
        v
versioned Dunit userspace ABI / libdunit low-level contract
        |
        v
Green Tea Kernel native syscalls and objects
```

- Green Tea Kernel не принимает Linux syscall numbers и не экспортирует Linux kernel structs.
- Публичные заголовки musl предоставляют переносимость исходного C-кода; это facade, а не kernel ABI.
- Внутренний adapter переводит POSIX flags, `errno`, paths, `stat`, directory entries, clocks и process operations в собственные Dunit contracts.
- x86_64 System V calling convention, ELF и стандартный initial stack допустимы как независимые отраслевые ABI; их использование не означает Linux compatibility.
- Если функция пока не поддерживается, она возвращает документированную ошибку (`ENOSYS`, `ENOTSUP`) или исключается из заявленного профиля. Заглушка, ложно возвращающая success, запрещена.

#### M6.2. Исходники и сопровождение fork

- [x] Создать отдельный репозиторий `dunit-musl` как fork полного upstream source tree, не копировать выборочные `.c` и headers.
- [x] Зафиксировать исходный upstream release/tag и commit в `UPSTREAM.md`.
- [x] Подключить fork в `toolchains/dunit-musl/` как pinned submodule либо воспроизводимый pinned checkout.
- [x] Вести Dunit-изменения тематическими commits: `build`, `crt`, `syscall`, `fs`, `vm`, `thread`, `time`. **Осталось (отдельные gates):** `signal`/`network`/`ldso`.
- [x] Хранить таблицу отличий от upstream и регулярно переносить bug/security fixes; обновление версии должно проходить полный libc test gate.
- [x] Не удалять из fork временно неподдерживаемые подсистемы: исключать их build profile, чтобы позднее не восстанавливать дерево вручную. (Неподдержанные syscalls → документированный `-ENOSYS` sentinel, ни один файл не удалён.)
- [x] Проверить лицензионные notices musl и генерировать перечень исходных версий в SDK/image metadata.

**Цель:** управляемый fork с проверяемым происхождением. **Причина:** маленький vendor snapshot быстро потеряет исправления и станет непереносимым. **Подсистемы:** source management, build, release. **Зависимости:** M0. **Результат:** любой Dunit release указывает точный upstream и Dunit commit. **Готовность:** clean checkout воспроизводит тот же `libc.a`; upstream delta автоматически формируется в CI. **Тесты:** clean/offline rebuild, checksum comparison, upgrade rehearsal. **Риски:** большой вечный diff; минимизировать OS-specific changes и не переформатировать upstream files.

#### M6.3. Dunit ABI прежде libc

До интеграции musl выпустить `Dunit Userspace ABI v0`:

- [x] Зафиксировать syscall calling convention: `rax` number; `rdi/rsi/rdx/r10/r8/r9` arguments; `rax` result.
- [x] Версионировать syscall numbers независимо от Rust `libdunit`; генерировать Rust и C definitions из одного manifest.
- [x] Определить signed error return range и стабильное преобразование Dunit errors в POSIX `errno`.
- [x] Определить lifetime и права handles/fds, наследование при spawn, reserved descriptors `0/1/2`.
- [x] Определить endian, width и alignment для scalar types, `time_t`, `off_t`, `ino_t`, `pid_t`, pointers и atomics.
- [x] Не раскрывать kernel-private `Stat`, process или directory structs: использовать versioned wire structs и переводить их в публичные musl types.
- [x] Определить process-entry ABI: initial stack, `argc/argv/envp`, auxiliary vector, page size, random seed и program headers.
- [x] Определить ELF contract: accepted types/segments, load bias, stack permissions, `PT_TLS`, `PT_GNU_RELRO`, `PT_INTERP` policy.
- [x] Добавить feature/capability query вместо определения поведения по версии kernel.

Для static C уже есть initial stack с argv/envp/auxv и Dunit `crt1`/`__libc_start_main` (`abi/README.md`, `kernel/src/elf/mod.rs`, `tools/m6_c_abi_markers.json`). Kernel не знает внутреннее устройство musl; дальнейшие entry ABI changes требуют отдельного согласования.

**Готовность ABI v0:** независимый C test без musl получает arguments/environment/auxv, вызывает raw syscalls и корректно завершает процесс. Изменения ABI до v1 допустимы только с bump версии и обновлением conformance tests.

#### M6.4. Cross-toolchain и sysroot

- [x] Ввести логическое target имя `x86_64-dunit`; не выдавать Dunit binary за `x86_64-linux-musl`.
- [x] Сначала использовать LLVM/Clang + LLD или небольшой compiler-driver wrapper; полный GCC port не является условием первого C binary.
- [x] Научить compiler driver выбирать Dunit linker script, crt objects, include paths, library paths и default static mode.
- [x] Собрать из fork `crt1.o`, `crti.o`, `crtn.o`, `libc.a` и public headers в staging sysroot.
- [x] Не смешивать host headers/libraries с Dunit sysroot; сборка с найденным host `/usr/include` должна падать.
- [x] Создать команды `x86_64-dunit-cc`, `x86_64-dunit-ar`, `x86_64-dunit-strip` и SDK manifest с ABI/libc versions.
- [x] Добавить CMake toolchain file, Meson cross file и Autoconf cache после стабильного прямого compiler flow. **Проверено:** генератор `tools/dunit_sdk.py`, `tools/test_m6_tools.py` (включая пути с пробелами/кавычками), native ELF gate. Это cross-files и provenance, а не весь M7 SDK.

Предлагаемый sysroot:

```text
toolchains/sysroot/
├── usr/include/          # musl public headers + Dunit extensions
├── usr/lib/
│   ├── crt1.o
│   ├── crti.o
│   ├── crtn.o
│   └── libc.a
├── system/include/dunit/ # native Dunit API outside POSIX namespace
└── share/dunit-sdk/      # ABI manifest, supported profile, licenses
```

**Готовность:** `x86_64-dunit-cc -static hello.c -o hello` создаёт ELF без host dependencies; `readelf` подтверждает ожидаемые segments/symbols, а Green Tea Kernel запускает его.

#### M6.5. Разделение кода внутри musl fork

x86_64 arithmetic, atomics, string/memory implementations и большая часть ISO C остаются upstream. Dunit-specific код концентрируется в узких границах:

```text
arch/x86_64/              CPU ABI, atomics, TLS access; минимум Dunit delta
crt/ + ldso/              Dunit process entry; dynamic loader позже
src/internal/             syscall/error/capability bridge
src/dunit/                native adapters and structure translation
src/thread/               Dunit thread/TLS/wait-wake backend
src/process/              Dunit spawn/wait; fork-dependent paths gated
src/mman/, src/fs/        native VM/VFS mappings
```

- [x] Сохранить upstream generic implementations там, где они не зависят от Linux semantics.
- [x] Не размазывать `#ifdef __dunit__` по всему дереву: вводить небольшие backend interfaces и Dunit translation units.
- [~] Аудировать прямые `SYS_*`, Linux ioctls/paths, cancellation и структуры. **Готово:** native adapters, generated ABI и negative/hardening tests. **Исправлено:** native spawn-actions open/close/dup2/chdir атомарны, неподдержанные attributes явно отклоняются; ioctl/fcntl имеют native adapters. system/popen явно ENOSYS. Полный broad-POSIX audit не закрыт; границы и inventory в `docs/m6-readiness.md` и `docs/libc-test.md`.
- [x] Генерировать `bits/alltypes.h` и public ABI types только после утверждения их размеров. **Готово:** `alltypes.h.in` upstream-stock, размеры ABI-типов — стандартный x86_64 LP64 (`off_t`/`time_t` 64-бит, `struct stat` 152, `termios` 60, `dirent` 280) и согласованы с адаптерами (аудит M6.5).
- [x] Dunit-specific public functions помещать в отдельные headers/namespace, не загрязнять POSIX имена. **Готово:** ноль Dunit-имён в public `include/`; адаптеры (`__dunit_*`) в reserved `__`-namespace, публичные символы остаются POSIX-именами (аудит M6.5).

#### M6.6. Фазы реализации

##### M6-A — freestanding bootstrap и static hello

- [x] Реализовать Dunit `crt1`/`_start`, libc initialization, `main` dispatch и termination.
- [x] Подключить `exit`, `_Exit`, `write`, минимальный error translation и descriptors `0/1/2`.
- [x] Собрать static-only musl profile: ISO C string/memory/math/conversion и минимальный stdio path.
- [x] Проверить constructors/destructors, `.bss`, stack alignment, argc/argv/envp и return from `main`.

**Результат:** статический `hello.c` и программа с arguments. **Готовность:** оба ELF собираются только через Dunit sysroot, загружаются штатным loader и завершаются с проверяемым exit code. **Тесты:** empty args, long args, environment, constructors, invalid user pointers, stdout/stderr. **Риски:** несовместимый entry stack, утечка host headers, ошибочная stack alignment.

##### M6-B — VM, allocator, time и entropy

- [x] Реализовать anonymous/private `mmap`, `munmap`, `mprotect`; alignment 4 KiB, zero-fill, partial unmap и запрет W+X проверены `[VM-TEST] OK`.
- [x] Адаптировать allocator musl без требования Linux `brk`; optional hints вроде `madvise` могут быть no-op только если контракт это разрешает.
- [x] Добавить realtime clock и time conversion. **Проверено:** CLOCK_REALTIME = валидный UTC timestamp Limine + monotonic, без timestamp — явная ошибка; `m6_musl_time_markers.json`, clock host test. Отдельный RTC driver не заявляется.
- [~] Добавить entropy interface для guards/криптографии. **Готово:** `AT_RANDOM` в ELF initial stack. **Осталось:** secure entropy API и доказанный источник — `kernel/src/elf/mod.rs` использует bootstrap TSC seed; его нельзя считать CSPRNG для TLS.

**Результат:** стабильные `malloc/calloc/realloc/free`, clocks и random seed. **Готовность:** allocator переживает fragmentation/OOM и возвращает корректный `errno`; protection faults детерминированны. **Тесты:** allocator stress, zero/huge allocations, map/unmap/protect boundaries, OOM, monotonicity, entropy availability. **Риски:** VM leaks, use-after-unmap, executable writable memory.

##### M6-C — files, directories и installed environment

- [x] Покрыть `open/read/write/close/seek`, `stat/fstat`, cwd, directories, rename/unlink/mkdir и `fsync`. Плюс `ftruncate` (Seek=80, Fstat=81, Truncate=84), `access` (Stat-based), `O_EXCL`/`mkstemp` и `rewinddir`.
- [x] Переводить POSIX open/mode flags и Dunit metadata через проверяемый adapter; не передавать public musl structs в kernel.
- [x] Реализовать buffered `FILE`, standard streams и terminal capability query.
- [x] Определить Dunit paths для users, configuration, temporary files, DNS и account database; не копировать Linux FHS автоматически. **Текущая политика путей:** `/system` — read-mostly образ ОС (бинарники, библиотеки, read-only ассеты); `/persist` — DunitFS, пользовательские данные и конфигурация (writable, переживает ребут); `/tmp` — MemFS, временные файлы (не переживают ребут); `/app`, `/assets` — данные образа initrd, загруженные в runtime MemFS (это ещё не enforced read-only disk root). DNS отложен до M8 (сеть); account database/ownership остаётся отдельным несетевым gate — Linux FHS намеренно не копируется.
- [~] Загружать C programs и libc artifacts с installed root. **Готово:** они в initrd/sysroot, `include_bytes!` C-программ в kernel нет. **Осталось:** настоящий installed disk root вместо MemFS.

**Результат:** C utility работает с DunitFS после установки. **Готовность:** create/write/fsync/reboot/read с тем же hash; stdio error/EOF semantics проходят тесты. **Тесты:** large/empty/sparse-policy files, Unicode bytes in paths, disk-full, rename atomicity, corrupted storage, terminal redirection. **Риски:** несоответствие `stat/dirent`, неполный fsync contract, скрытые fixed paths musl.

##### M6-D — process model без обязательного `fork`

- [x] Спроектировать `spawn/exec-image/wait/kill` с arguments, environment, cwd и явным inheritance handles. (SpawnArgs=79 с argv/env, WaitProcess=16, KillProcess=18; привилегии наследуются только через kernel-spawn, не через posix_spawn.)
- [x] Переписать musl `posix_spawn` backend на прямой Dunit spawn contract, не эмулировать его через Linux `clone/vfork`.
- [x] Реализовать pipes, descriptor duplication и spawn redirection. **Проверено:** native Pipe/Fcntl/Poll, shared status и CLOEXEC; `m6_pipe`, `m6_dup`, `m6_profile`, `m6_native_fd` проверяют EOF/nonblock/ordered spawn actions и неизменность родительских fd/cwd. Неподдержанные attributes отклоняются до публикации ребёнка.
- [x] Возвращать `ENOSYS` для `fork`, пока нет корректных COW, multithreaded semantics и `pthread_atfork`.

**Результат:** C parent запускает C child и получает exit status. **Готовность:** spawn file actions, environment replacement и pipe redirection работают без fd leaks. **Тесты:** missing executable, permission denial, child fault, concurrent children, inherited/closed handles. **Риски:** deadlocks при spawn, неявная передача privileged handles, попытка приложений обойти отсутствие fork.

##### M6-E — threads, TLS и synchronization

- [x] Green Tea Kernel создаёт schedulable user threads с отдельными kernel/user stacks и общим address space.
- [x] Поддержать установку/переключение x86_64 `FS.base` и initial static `PT_TLS` image; dynamic thread vector contract остаётся частью будущего dynamic TLS/ldso profile.
- [x] Реализовать thread create/exit/join/detach и безопасное освобождение stack/TLS после завершения.
- [x] Реализовать Dunit `wait_on_word/wake` с atomic compare-and-block, timeout и защитой от lost wakeups; не копировать Linux futex ABI.
- [x] Адаптировать musl pthread create/join, mutex, condvar, rwlock, once и per-thread `errno`.
- [~] Сначала запретить asynchronous cancellation; cancellation points включать только вместе с формально определённой interruption model. **Готово:** pthread_cancel и asynchronous cancellation явно возвращают ENOTSUP без побочных эффектов (`m6_profile_markers.json`); native waits не обещают interruption. **Осталось (позже, M6-F):** включение cancellation points вместе с signal/interruption-моделью.

**Результат:** настоящий pthread subset поверх native Dunit threads. **Готовность:** уникальный TLS/errno у каждого потока, blocked mutex не расходует CPU, join/detach не течёт. **Тесты:** TLS isolation, 1000 create/join cycles, mutex/condvar contention, timeout race, owner death policy, preemption stress, allocator under threads. **Риски:** ABI-зависимый layout musl TCB, lost wakeups, освобождение живого TLS, scheduler/VM lock inversion.

##### M6-F — signals, networking и dynamic linking (отдельные gates)

- [ ] Signals начинать только после Dunit signal/event model: delivery frame, masks, restart/interruption, alt stack и thread targeting.
- [ ] Не заявлять `pthread_cancel`, timers и полноценные signals, пока их скрытые зависимости не проходят race tests.
- [ ] Sockets добавлять после native Dunit network handles; adapter переводит POSIX socket API, но kernel сохраняет собственную object model.
- [ ] Dynamic loader вынести в отдельный milestone: `PT_INTERP`, DSO mapping, relocations, symbol lookup, RELRO, TLS modules, `dlopen/dlsym/dlclose`.
- [x] До dynamic tier официальные Dunit C-программы собираются static через Dunit sysroot; отсутствие `.so` не является дефектом static-first M6. Это политика сборки, не заявление о готовом dynamic tier.

`fork` не блокирует static-first port: сначала Dunit-native spawn/`posix_spawn`; `fork` реализовать позже через COW только если реальный software demand оправдает сложность.

#### M6.7. Минимальные kernel primitives и порядок появления

| Уровень | Обязательные primitives | Что открывает |
|---|---|---|
| A | exit, write, stable entry ABI, errors | crt, hello, basic stdio |
| B | mmap/munmap/mprotect, clocks, sleep, entropy | malloc, time, guards |
| C | complete file/dir metadata and fsync | stdio, installed C tools |
| D | spawn/exec-image/wait, pipes, dup/inheritance | process utilities |
| E | preemption, thread lifecycle, FS.base/TLS, wait/wake | pthread subset |
| F | events/signals, sockets, DSO VM support | broader POSIX, network, `.so` |

`mmap` должен быть нормальным VM contract, а не allocator-specific syscall. TLS base является состоянием **потока**, а не процесса. `wait_on_word` обязан атомарно проверять значение и усыплять поток. Эти три решения одновременно нужны GUI services и musl, поэтому реализуются в M1 один раз как общие kernel primitives.

#### M6.8. Поддерживаемый libc/POSIX profile

Для каждого release публиковать машинно-читаемый capability manifest и таблицу:

| Профиль | Содержание | Статус первого порта |
|---|---|---|
| ISO C core | strings, memory, math, conversion, stdio subset | must-have |
| Dunit static runtime | crt, errno, allocator, environment, files, clocks | must-have |
| POSIX process subset | spawn, wait, pipes, fd actions | should-have |
| pthread subset | TLS, create/join, mutex/condvar/once | после M1 |
| signals/fork | только после формальной process model | later |
| sockets | после native network stack | later |
| dynamic linking | loader/DSO/TLS modules | separate milestone |

Заявление «musl портирована» запрещено без указания profile/version. M6 завершается на static C + filesystem + pthread subset; broad POSIX и dynamic linking имеют отдельные критерии.

#### M6.9. Тестирование и release gate

- [x] Добавить self-checking libc ELF: существующая конвенция `userspace/ctests/` + `tools/m6_*_markers.json` покрывает startup, stdio, malloc, files, clocks, spawn, базовые pthread/condvar. Barrier/прочие отсутствующие тесты из старой ветки не включены в этот `[x]`.
- [x] Запускать все kernel/OS тесты только через `tools/qemu_test.py`; каждый тест выдаёт стабильный serial marker и exit status.
- [x] Подключить релевантный subset `libc-test`; unsupported cases перечислять с причиной и владельцем. **Проверено:** 12 неизменённых upstream cases, pinned commit/hashes, полный inventory 435 cases; `tools/libc_test_markers.json`, `docs/libc-test.md`. Это subset, не заявление о прохождении всей libc-test.
- [~] Добавить negative tests: invalid pointers, invalid UTF-8-neutral path bytes, bad fds, OOM, interrupted waits, child crash. **Готово:** invalid pointers/bad fds/OOM/child-crash покрыты (musl_harden, fault_child, nx_child, musl_reap/slow_child). **Осталось (позже, M6-F):** interrupted waits — нужна signal/interruption-модель.
- [x] Проверять ELF статически: target machine, headers, executable stack, unexpected DYNAMIC/INTERP, unresolved symbols. **Проверено:** `tools/check_userspace_elf.py` в Makefile до initrd; включая explicit RW/NX GNU_STACK для TCC и запрет W+X.
- [~] Для каждого ABI/libc change пересобирать sysroot и C apps. **Готово:** `tools/ensure_musl_sysroot.py` проверяет fingerprint исходников/компилятора; исправлено повторное использование старой libc после правок fork (Lua I/O regression). **Осталось:** previous-release binary compatibility suite и замороженный ABI release contract.
- [~] Разделить gates: single-thread static, filesystem/persistence, pthread/preemption, network, dynamic. **Готово:** раздельные marker-suite single-thread static / filesystem+persistence / pthread+preemption. **Осталось:** network (M8) и dynamic (позже) gates.

**Общий результат M6:** native Dunit-linked C programs и SDK без Linux kernel ABI. **Критерий завершения:** reproducible `x86_64-dunit` sysroot; static hello/args/env/file/malloc/spawn/thread suites проходят в QEMU; persistence case переживает power cycle; unsupported POSIX surface документирован. **Главные риски:** скрытые Linux assumptions, слишком ранняя заморозка ABI, некорректный TLS, попытка одновременно реализовать signals/fork/ldso. **Меры:** static-first, capability profiles, узкий adapter, фазовые test gates и контролируемый upstream delta.

Проверка доведения M6 (2026-10-24): исходные pipe FAIL исправлены;
core-suite прошёл отдельно; итоговый один regression-прогон подтвердил
40 полных контрактов / 73 команды / 175 required markers / 0 forbidden. Native
1000 join + 1000 detached подтвердили TLS/errno/malloc/yield и reclamation;
DunitFS 128 KiB write/fsync/остановка VM/повторная загрузка/readback подтвердил
hash `b4c071280fb62325`. Полный список доказательств и незакрытых gates:
[docs/m6-readiness.md](docs/m6-readiness.md). В текущем ABI generator check:
88 syscalls + 25 errno + 7 rights. Secure entropy, signals/interruption,
dynamic tier и disk-backed system root этим не закрываются.

### M7 — Dunit SDK (should-have после M6; блокирует M8–M10)

SDK — слой, на котором пишутся прикладные программы Dunit: безопасные Rust-обёртки над `libdunit`, декларативный GUI на базе существующего UI-runtime (`runtime/` — `dunit_ui/dunit_style/dunit_text/dunit_widgets/dunit_render`), инструменты сборки/запуска/тестов и «красиво по умолчанию». Программы собираются **на хосте** (кросс в `x86_64-dunit`, host-компилятор = кросс-компилятор, механизм тот же); компиляция Rust внутри Dunit планируется, но тяжёлая (сначала нужен rustc-порт — отдельный поздний трек, не блокирует SDK).

Три уровня программ (см. §6/Makefile `USERSPACE_APPS`+`MUSL_CTESTS`):
1. **Низкоуровневые тесты ядра/ABI — ОСТАЮТСЯ на `libdunit`, НЕ мигрируют** (проверяют ядро, а не SDK): `abi_test, c_hello, elf_demo, fault_pf, fault_ud, vm_test, vm_peer, vm_guard_fault, vm_protect_fault, futex_test, handle_test, runtime_stress, tls_test, preempt_child/test, resumable_child/test, scheduler_test, spawn_ready_test, kill_target, thread_test, wait_test, yield_child/test, ipc_child/parent, args_test, env_test, cwd_test, path_test, exit_test, stdin_test, file_api_test, fs_test, input_test, color_test, image_demo, gui_shbuf_peer, gui_client, dsh_calc_test, pty_echo/test, termios_test, init_probe` + все `MUSL_CTESTS` C-фикстуры + `tcc_hello`.
2. **Системные сервисы — только на НИЖНИЕ слои SDK (рантайм, безопасные обёртки, общий крейт протокола), НЕ переписываются целиком**: `gui_server`, `init`.
3. **Приложения — мигрируют на SDK полностью, по одной**: `dsh, calc, dufetch, dtop, fsck_dunit, bmp_viewer, gui_calc, gui_files, gui_terminal, gui_stat, gui_demo, gui_settings`. Миграция — НЕ копирование старого вида: если SDK даёт красивые кнопки/поля, то калькулятор и прочее делаются реально красивыми, минималистичными и качественными.

#### M7.1. wait-set — единый примитив ожидания (ПЕРВЫЙ пункт; изменение `abi/*.abi` → Решение человека)

Прежние вызовы IPC → WaitEvent(33), память → FutexWait(41), ребёнок → WaitProcess(16) и native Poll(87) сохранены. По решению пользователя 26 октября 2026 добавлен WaitSetWait(89): один call-scoped level-triggered набор IPC/timer/file/pipe/PTY/child-exit. Контракт и ограничения: `docs/sdk/m7-start.md`. Полный M7.1 остаётся открытым до edge/persistent registration и расширенной интеграции.

- [~] Реализованы снимок набора {own IPC inbox, timer/deadline, file/pipe/PTY readiness, child pid}, блокирующее ожидание, относительный таймаут, токены и отмена через IPC. Сохраняются исходные deadline и fd slot identity при пробуждении. Readiness-edge и постоянная регистрация ещё не реализованы; sockets отложены до M8.
- [x] `WaitSetWait` (89) добавлен через `abi/syscalls.abi` и регенерацию Rust/C outputs после решения пользователя 26 октября 2026; совместимость прежних вызовов сохранена.
- [~] Добавлены безопасная `libdunit::wait_set::wait` и `dunit::event::EventLoop`, применённый в `gui_stat`. Готовность fd разделяет реализацию с native Poll; старые futex/IPC ABI остаются совместимыми. Полная унификация остальных SDK циклов ещё впереди.

**Цель:** один примитив ожидания для GUI/сетевых/сервисных циклов. **Зависимости:** M1 wait queue (`block_current(pid, deadline)` уже умеет IPC+timer). **Результат:** программа блокируется на наборе объектов одним вызовом. **Критерии приёмки (qemu_test):** тест регистрирует {pipe, IPC, timer}, пишет в один из них → просыпается именно на нём с корректным токеном; таймаут срабатывает; закрытие объекта будит с hangup-событием; маркер `[WAITSET] OK`. **Риски:** переусложнение API и lost-wakeup — сначала минимальный набор классов + property-тест на гонки. **DoD:** wait-set в ABI, обёрнут в SDK, используется event loop (M7.4) — не «добавлен и не вызывается».

#### M7.2. Модель приложения и инструментарий

- [~] `cargo dunit new / build / run / test` (CLI/TUI/GUI реализованы; standalone SDK ещё открыт, см. `docs/sdk/cargo-dunit.md`): `build` кросс-собирает в `x86_64-dunit`; `run` собирает, кладёт бинарник в initrd, грузит QEMU через `tools/qemu_test.py` и запускает приложение; `test` гоняет тесты в QEMU и САМ формирует маркеры (разработчик их не пишет).
- [~] Манифест приложения `dunit.toml` v1 принят пользователем 26 октября 2026 (`docs/sdk/app-manifest-proposal.md`); строгая проверка, SDK-упаковка и GUI discovery реализованы (`docs/sdk/desktop-catalog.md`): `id`, имя, иконка, вид (`gui`/`tui`/`cli`), `capabilities`. Launcher/dock/startup читают общий каталог в userspace, без `match app`. CLI/TUI запуск с desktop через терминал и миграция legacy-регистрации ещё открыты.
- [~] `#[dunit::main]` реализован: loader entry, argv/env init, общий panic handler и exit 0/1/101; применён в `calc`, `gui_stat` и гостевом SDK API-тесте. `#[dunit::test]` реализован через compiler-collected native harness, возвращает ошибку при пустом наборе, Err и panic.
- [~] Безопасные `fs, process, time, io, env, args` и существующий `system::stats` реализованы поверх `libdunit`, с общим `Error` и generated errno. `File` владеет дескриптором, буферизованный ввод сохраняет строки, Child ждёт через wait-set. `calc` не содержит `unsafe`, `extern "C"`, raw entry или libdunit dependency. Thread/sync wrappers ещё нет; `docs/sdk/native-api.md`.
- [ ] `dunit::net` — точка расширения без реализации стека (стек = M8).

**Цель:** писать приложения без ручного ABI и `unsafe`. **Зависимости:** M6 (musl/dunit-cc toolchain), M7.1 (event loop нужен wait-set), M7.3 (рантайм). **Результат:** `cargo dunit new hello && cargo dunit run` показывает окно/вывод на ядре. **Критерии приёмки (qemu_test):** сгенерированный шаблон приложения собирается, пакуется, стартует и выдаёт маркер `[SDKAPP] ok` без единой строки `unsafe` в его исходнике (греп по шаблону = 0 `unsafe`/`extern "C"`); `cargo dunit test` формирует и проверяет маркеры сам. **Риски:** инструмент дублирует логику `tools/qemu_test.py` — обёртывать его, не форкать. **DoD:** хотя бы одно реальное приложение (M7.11) собрано и запущено только через `cargo dunit`.

#### M7.3. Рантайм SDK — развилка (Решение человека)

Два варианта несущего рантайма приложений: **(A)** собственный `no_std + alloc` крейт с нативным Dunit-API (как сегодняшние `runtime/` + `libdunit`); **(B)** полноценный Rust `std` поверх musl-порта (`std` требует бэкенд: процессы/потоки/сеть/время/fs поверх Dunit). Разбор обоих — в разделе «Решения человека»; агент не выбирает сам.

- [x] Зафиксировать выбор (A/B) решением человека ДО написания прикладного API. **Решение 26 октября 2026:** A, `no_std + alloc` поверх `libdunit`; пользователь выбрал этот вариант для M7 SDK.
- [~] Начат узкий `runtime/dunit` (`no_std + alloc`): owning event loop поверх `libdunit::wait_set`, используется `gui_stat`. Добавлены fs/process/time/io/env/args API и `#[dunit::main]`; `calc` использует их без ручного ABI. Добавлены native `#[dunit::test]`, CLI/TUI/GUI cargo tooling и общий window helper. Реализованы typed root Component/update/view и сквозной damage; Добавлены интерактивные push/toggle/checkbox/radio/switch и одно-/многострочное поле ввода; thread/sync и остальные виджеты ещё не реализованы.

**Цель:** одна несущая модель для всех приложений. **Зависимости:** M6 (для B — зрелость musl: TLS/threads/fs/time уже есть; не хватает сети и части POSIX). **Результат:** приложения SDK используют единый рантайм. **Критерии приёмки:** эталонное приложение собирается и проходит тесты на выбранном рантайме; документирован профиль (что доступно). **Риски:** `std`-путь тянет широкую POSIX-поверхность (сеть/signals) раньше времени; `no_std`-путь — свой API, который придётся развивать. **DoD:** выбор зафиксирован, второй вариант не висит полуреализованным.

#### M7.4. Декларативная GUI-модель

Большинство старых клиентов ещё вручную собирают DUI+DSS строку и красят весь surface. `gui_stat` переведён на typed Component/update/view поверх существующих `dunit_ui`/`dunit_style`/`dunit_render`; CPU canvas, shared-buffer upload и compositor используют damage через существующий gui-v1 без изменения wire-формата.

- [x] Typed root `Component` с `update(Msg)` и `view() -> View<Msg>`; `Application` владеет моделью и выдаёт только immutable access. `gui_stat` использует этот слой без DUI string builder; [контракт и проверки](docs/sdk/components.md). Вложенный lifecycle компонентов и поведение виджетов остаются отдельной работой.
- [x] CPU diff дерева/styles/layout, dirty bounding rectangle и clipped repaint, включая очистку старого текста и compositing перекрытий. Два shared backing переиспользуются после RELEASE+DESTROY с repair предыдущего и нового damage; неизменившийся кадр пропускается. Композитор выбирает accepted committed frame, учитывает stride/offset, обновляет только dirty texture и частично компонует/выводит стабильные непрозрачные, немасштабированные, неперекрытые области. Scene changes, эффекты, прозрачность и перекрытия дают полный fallback. QEMU проверяет dirty row 480×32: texture/composed/presented по 15 360 пикселей, повторное использование и отсутствие лишнего commit при no-op.
- [~] Общий `dunit::window` helper поверх gui-v1 и wait-set: configure/resize, кадры, input/close, освобождение буферов; используется `gui_stat` и GUI-шаблоном `cargo dunit`. Конкурирующий CONFIGURE восстанавливается через destroy/recreate поверхности с повторной отрисовкой; остальные старые клиенты ещё требуют миграции; [контракт и проверки](docs/sdk/window.md).
- [x] Тестирование без QEMU: последовательность `Msg` проверяет состояние и дерево `view`; pixel tests сравнивают incremental repaint с полным кадром. Native `[SDKUI] ok` проверяет настоящий keyboard input, dirty row и сохранение модели при maximize/restore.
- [~] Состояние SDK push/checkable buttons и полей ввода (`hover/focus/pressed`, cursor/selection/history) сохраняется по стабильному ключу, виду и radio-группе при перестроении typed View, перестановке узлов и resize; `checked` всегда читается из новой модели. Удаление, смена вида и disabled сбрасывают состояние; повторное добавление не восстанавливает удалённый экземпляр. QEMU проверяет настоящий ввод и сохранение фокуса после reorder/resize; [контракт](docs/sdk/buttons.md). Остальные типы виджетов и вложенный lifecycle ещё открыты.

**Цель:** приложения описывают UI декларативно, состояние меняется только в `update`. **Зависимости:** M7.1 (event loop), M7.3 (рантайм), `runtime/` крейты. **Результат:** приложение = `Component`; SDK сам гоняет parse→layout→diff→paint→commit. **Критерии приёмки:** (1) unit-тест без QEMU: последовательность `Msg` даёт ожидаемое дерево/состояние; (2) qemu_test: приложение на `Component` рисуется и реагирует на ввод, маркер `[SDKUI] ok`; (3) при изменении одного узла damage-область покрывает только его (не весь экран) — проверяется счётчиком перекрашенных пикселей/маркером. **Риски:** diff-слой сложен — начать с поддерева-гранулярности, не попиксельно. **DoD:** минимум одно мигрированное приложение (M7.11) использует `Component`, строковая сборка UI в нём удалена.

#### M7.5. «Красиво по умолчанию» (проверяемо, не на вкус)

- [~] Готовая тема без прикладных стилей: `ComponentWindow::open_themed`, Text и контейнеры, типографика/отступы из общих DSS-профилей. System Monitor и GUI-шаблон используют этот путь; private DSS/ручной font loader монитора удалены. Golden базового компонента и галереи эффектов сверяет реальные framebuffer pixels; полный набор виджетов и эталонных приложений ещё открыт.
- [~] Запрашиваемый runtime-реестр DSS `Tokens` хранит типизированные именованные значения, `parse_with_tokens` строго проверяет ссылки. SDK использует общие цвета, размеры шрифта, отступы и radius/border; Дополнительные cards.dss/glass.dss используют токены теней, размытия, цвета и радиуса. Motion используется для SDK paint-property transitions. Осталось применить токены к будущему полному набору виджетов и подключить SDK-анимации. [Контракт](docs/sdk/themes.md).
- [~] Светлая и тёмная темы. **Готово:** desktop TOML-пресеты, terminal themes, setup/Settings import; SDK dark/light DSS, `[desktop] sdk_theme`, автоматическая перекраска открытых themed ComponentWindow с сохранением модели и последнего рабочего кадра при ошибке. Native golden проверяет dark/light/restore с закреплённым шрифтом; kernel terminal соблюдает display lease и не перезаписывает GUI при serial-выводе. **Осталось:** миграция остальных клиентов, расширение визуальных эталонов и объединение декораций/SDK-темы в M7.9; ручные canvas-клиенты не получают автоматический reload.
- [x] `dunit_render`: скруглённые заливки/внутренние границы, градиенты, тени, backdrop-blur внутри SDK canvas и прямоугольные/скруглённые group clips. Coverage/interpolation/box-filter kernels общие с `gui_server`. Damage покрывает старую/новую тень; изменения вне старых/новых blur-прямоугольников и области выборки сохраняют частичную перерисовку. Пересечение с blur пока перестраивает клиент целиком; no-op пропускается. Отдельный native golden проверяет фон под blur, clipping, reload/rollback и точное восстановление; [контракт](docs/sdk/themes.md). Blur самого окна поверх desktop и бюджет кадра остаются отдельными пунктами ниже.
- [~] Анимации/переходы с бюджетом стоимости. **Готово:** desktop animation clock, TOML timing и Lua tiling policy в отдельном процессе с deadline (`userspace/system_apps/gui_server/src/animation.rs`, `lua_policy.rs`, `tools/lua_desktop_markers.json`). **SDK:** классы View, timed transitions для цветов/radius/border-width/blur через `dunit_style::motion`, wait-set sampling, retarget по stable key, reduced motion и точные native endpoint goldens; [контракт](docs/sdk/motion.md). **Осталось:** анимации полного набора widgets/геометрии и доказанный frame budget для целой сцены.
- [ ] Прозрачность, blur и скругление ОКНА доступны приложению декларативно, без работы с ARGB.
- [~] Иконки/шрифты и resize. **Готово:** DejaVu TTF, native static SVG runtime, общий `dunit_image::IconTheme`, все пять полных Chameleon ZIP-наборов; TOML source/alias/fallback manifests, index.theme size/scale lookup, dock/launcher/switcher/Files. Старые Aqua/Breeze сохранены для совместимости. `tools/svg_markers.json` проверяет пять тем и последовательные config roundtrips; `tools/display_3k_markers.json` — режимы до 3072×1920. **Осталось:** упаковка через SDK manifest, адаптивные SDK components и SVG-функции вне документированного static profile; доступность архива не означает поддержку всех SVG-фич его 58518 иконок.

**Цель:** приложения выглядят целостно и качественно без ручного стайлинга. **Зависимости:** M7.4, `dunit_render`, эффекты `gui_server`. **Результат:** дефолтная тема + токены + анимации применяются ко всем SDK-приложениям. **Критерии приёмки (НЕ вкусовые):** (1) набор эталонных приложений + **визуальные тесты** — снимок окна в QEMU сравнивается с эталоном (pixel/hash-diff), обновление эталона только осознанным коммитом; (2) смена темы light↔dark перекрашивает эталон целиком (diff ≠ 0 во всех токен-зонах); (3) **замер времени кадра** как serial-маркер (`gui_server` уже печатает `frame ~N ms`), сцена с несколькими прозрачными окнами держит бюджет ≤16 мс; (4) грэп: ноль литеральных hex-цветов/размеров в коде виджетов SDK. **Риски:** визуальные тесты хрупки — фиксировать шрифт/масштаб/детерминированный рендер. **DoD:** приложение без стилей проходит визуальный эталон; токены — единственный источник цвета.

#### M7.6. Набор виджетов — поведение В КОДЕ (как Qt/KDE), не только в стиле

DSS отвечает ТОЛЬКО за внешний вид и обязан адресовать состояния (`:hover :pressed/active :focused :disabled :checked` и т.п. — селекторы состояний уже работают). Поведение (клик, редактирование, навигация, перетаскивание) живёт в коде виджета. Базовые механизмы `dunit_widgets` используются typed SDK: push/toggle/checkbox/radio/switch получают общий `Interaction`, включая `checked`, измерение и DSS-отрисовку; поведение подключено через `Application`/`ComponentWindow`. Drag-выделение и колесо подключены для полей ввода; общий двухосевой scroll controller/clip/bar подключён к контейнерам и Textarea; остальные семейства виджетов ещё требуют реализации.

Для КАЖДОГО виджета описать и реализовать: состояния и переходы, реакцию на мышь и клавиатуру, фокус, доступность (роль + подпись для будущих a11y/IME), поведение при ресайзе.

- [~] Обычная SDK push button реализована: `View::button`, hover/active/focus/disabled из общей DSS-темы, click на отпускании, отмена при уходе/Leave/Esc, Tab/Shift+Tab с пропуском disabled, активация cooked Space/Enter, роль и подпись доступности. 7 новых host-тестов и QEMU `[WIDGETS] buttons ok` проверяют поведение, reorder/resize и настоящий ввод. Добавлены controlled toggle/checkbox/radio/switch: checked берётся из модели, общие DSS-индикаторы, роли/подписи/checked/pressed metadata; radio-группы проверяют единственность выбора, дают один Tab-stop и навигацию стрелками с wrap/disabled skip. Host-проверки и QEMU `[CHECKS] ok` проверяют четыре вида через клавиатуру и указатель, групповой фокус и resize; [контракт](docs/sdk/checkable-controls.md). Удержание клавиши как отдельное pressed-состояние требует доставки key-up, которой текущий control-канал не предоставляет. [Контракт и ограничения](docs/sdk/buttons.md).
- [~] Однострочное `View::input` подключено к typed SDK: controlled текст, UTF-8 scalar boundaries, курсор и мигание через wait-set, выделение клавиатурой/drag мышью, raw Backspace/Delete и word-навигация, undo/redo (64 snapshots), placeholder/password/read-only/disabled, лимиты и горизонтальная прокрутка. При отклонении правки моделью сохраняются прежние курсор/выделение/история. Host-проверки и QEMU `[INPUT] ok` проверяют реальный ввод, таймер, history, scroll, password/limit и resize; [контракт](docs/sdk/text-input.md). Добавлен `View::text_area`: LF split/join, Up/Down с сохранением scalar-колонки, Shift-выделение между строками, Home/End строки и Ctrl+Home/End документа, двумерный hit-test и вертикальная прокрутка курсором/колесом из DSS `scroll-step`; blink не сбрасывает ручной viewport. Host-проверки и QEMU `[MULTILINE] ok` проверяют editing/history, wheel, drag между строками и resize; [контракт](docs/sdk/multiline-input.md). Soft wrap, double/triple click, grapheme/IME, pointer modifiers и системный clipboard (M7.7) ещё открыты.
- [~] Общий `ScrollAxis` используется текстовым viewport и новой `View::scroll`: вертикальный контейнер естественной высоты, clipping и translated hit-test, вложение и wheel bubbling на границах, scrollbar thumb drag/track paging, Home/End/PageUp/PageDown/стрелки, Tab reveal, сохранение по ключу и clamp при resize/shrink. DSS задаёт gutter/track/thumb/min-thumb/step; полоса скрывается без overflow, фиксированный gutter исключает скачок раскладки. Host geometry/pixel tests и QEMU `[SCROLL] ok` проверяют реальный ввод, scrollbar и scrolled button. [Контракт](docs/sdk/scroll-areas.md). Добавлены `ScrollAxes::{Vertical,Horizontal,Both}`, горизонтальные drag/track/keyboard, независимый clamp/reveal обеих осей и общие полосы в Textarea. Барные жесты и reserved gutters сохраняют текстовое выделение/undo и ручной viewport при фокусе/мигании; слишком маленькое поле скрывает полосы без перезаписи границы. QEMU `[SCROLLXY] ok` проверяет обе оси, реальные track/drag, textarea selection/blink/edit/undo/shrink/resize. Добавлена автопрокрутка выделения у края Input/Textarea: неподвижный захваченный указатель расширяет выделение по wait-set таймеру; DSS задаёт интервал/шаг, Up/Leave/focus/disable и границы содержимого останавливают повторы. [Контракт](docs/sdk/selection-autoscroll.md). Timed auto-hide и виртуализация списков ещё открыты.
- [ ] Slider, spinbox, progress, индикатор занятости.
- [ ] Выпадающий список, меню, контекстное меню, меню-бар, подсказки (tooltip) с задержкой.
- [ ] Вкладки, сплиттер, панель инструментов, статус-бар, модальные диалоги, окно сообщения.
- [ ] Списки, деревья и таблицы с виртуализацией (рисуются только видимые строки), моделями выделения (одиночное/множественное/диапазон), сортировкой, переименованием на месте.
- [ ] Диалог выбора файла (на базе файлового менеджера `gui_files`).
- [ ] Система фокуса: порядок обхода Tab, кольцо фокуса, горячие клавиши/мнемоники, захват указателя, маршрутизация и всплытие событий, таймеры (`focus_order`/`event_path`/`hit_test` уже есть — подключить).
- [ ] Раскладка: flex и grid, min/max размеры, растяжение, выравнивание (в `dunit_ui::layout` уже есть — расширить состояниями виджетов).

**Цель:** полноценный набор виджетов с настоящим поведением. **Зависимости:** M7.4 (Component/event loop), M7.5 (токены/темы), M7.7 (буфер обмена для полей ввода). **Результат:** приложения строятся из готовых виджетов, а не рисуют прямоугольники. **Критерии приёмки:** per-widget тесты БЕЗ QEMU на событиях (подать PointerDown/Up/Move/Wheel/Key → проверить состояние/значение/фокус: кнопка активируется Space/Enter и click; press отменяется уходом курсора; поле редактируется и выделяется; slider меняется стрелками и drag; список навигируется стрелками; таблица/список с 10⁴ строк рисует только видимые — счётчик отрисованных строк ограничен viewport'ом) + qemu_test «витрина виджетов» с маркером `[WIDGETS] ok`. **Риски:** объём; делать по группам, каждая со своими тестами, не всё сразу. **DoD:** мигрированные приложения (M7.11) используют эти виджеты вместо ручной отрисовки; старый ручной hit-test из клиентов удалён.

#### M7.7. Drag-and-drop и буфер обмена — системный слой (посредник = `gui_server`)

Сегодня системного DnD и буфера обмена нет (в `gui_protocol_v1` — явный non-goal v1; у `gui_files` есть лишь внутрипроцессный `clip`). Это новый системный слой: приложения его НЕ реализуют — посредничает композитор.

- [ ] Посредник `gui_server`: захват указателя, значок под курсором, подсветка цели, доставка drop (живой pointer-цикл уже трекает `drag`/`pressed_win` — расширить до drag source/target поверх границы `in_shell`).
- [ ] Типизированное согласование: источник объявляет несколько представлений одних данных (список файлов, текст, изображение, свой тип), приёмник объявляет принимаемое, композитор выбирает лучшее. Типы описываются ОДИН раз в `gui_protocol_v1` и генерируются (как ABI).
- [ ] Действия: копировать/переместить/ссылка; модификаторы; приёмник сообщает допустимые.
- [ ] Данные отдаются ПО ЗАПРОСУ приёмника (на drop), не при начале перетаскивания. Полезная нагрузка не влезает в 256-байтный пакет протокола → shared-buffer transfer (зеркалить IMPORT_BUFFER+CAP1 capability-dance).
- [ ] Буфер обмена использует тот же слой и те же типы.
- [ ] Крайние случаи: множественный выбор, отмена по Esc, drop на цель, не принимающую тип, перетаскивание между рабочими столами, падение источника/приёмника посреди операции.
- [ ] Полная совместимость компонентов: файл/папка из `gui_files` в `gui_terminal` вставляет путь с правильным экранированием; папка в папку — перемещение/копирование; текст из любого приложения в любое поле ввода; файл на значок в dock открывает его в приложении.
- [ ] **Модель drop — Решение человека:** что получает приёмник — путь или capability на файл. Предлагаемый вариант: путь для текстовых приёмников, ограниченный по правам handle (на время операции) для остальных.

**Цель:** DnD и буфер обмена работают между любыми приложениями, единообразно. **Зависимости:** M7.8 (централизованный протокол), shared-buffer path (есть). **Результат:** перетаскивание и copy/paste пересекают границы процессов через композитор. **Критерии приёмки:** **матрица «каждый источник × каждый приёмник»** с ожидаемым результатом в каждой ячейке; автоматические тесты на синтетических событиях указателя (qemu_test) проверяют каждую ячейку; **новое приложение без записи в матрице валит тест**; маркеры `[DND] <src>-><dst> ok`; отмена Esc и drop на неподходящую цель не доставляют данные. **Риски:** огромный security/edge surface — данные по запросу, строгие типы, корректное поведение при падении участника. **DoD:** `gui_files` использует системный буфер обмена вместо своего `clip`; матрица зелёная.

#### M7.8. Протокол и магические числа — централизация/генерация

Живой control-канал `CAP1/INP1/RLD1/NOT1` теперь описан в `gui_protocol_v1::control`; `gui_server` и все 7 клиентов используют общий codec без изменения wire-байтов. `GUIC` остаётся централизован в `libdunit`. Формальный `DGUI`-роутер ввода используется лишь в self-тестах.

- [x] Раскладки сообщений и магические числа (`CAP1/INP1/RLD1/NOT1`, input/notify/reload) вынесены в `gui_protocol_v1::control`; дубли из `gui_server` и клиентов удалены. Общий Rust codec — единственный источник формата; отдельный генератор для этих фиксированных Rust-сообщений не нужен. 68 host-тестов протокола, QEMU `gui_smoke`, `settings_ui` на DWM ISO и `dwm_engine_v2` прошли 26 октября 2026.
- [x] Сохранён существующий control-канал из одного источника: `gui_protocol_v1::control`. Изменения версии/формата протокола и ABI не потребовались.
- [ ] Задействовать damage протокола в live-цикле (`present_damaged_frame` в self-тесте уже умеет partial `fb_present`) — префикс к DnD-курсору и декларативному diff (M7.4).

**Цель:** один источник правды для протокольных констант, без дублей. **Зависимости:** M7.4 (event loop поверх протокола). **Результат:** клиенты и `gui_server` импортируют раскладки из крейта/генерации. **Критерии приёмки:** грэп по дереву — определения `CAP1/INP1/RLD1/NOT1` остаются в ОДНОМ месте (0 дублей в клиентах); регрессия gui_smoke зелёная; damage в live-цикле уменьшает перекрашенную площадь (маркер/счётчик). **Риски:** смена формата ввода ломает все клиенты — менять за один заход с регрессией gui_smoke. **DoD:** дубли констант удалены, не «добавлен крейт, а дубли остались».

#### M7.9. Решение по `gui_server` (принято)

- Ядро компоновщика (композитинг, damage tracking, blur, прозрачность, маршрутизация ввода, посредничество DnD) **НЕ переписывается** на слое виджетов SDK. Переписывание не нужно и опасно для производительности (композитор программный, ядро одно).
- **Единый стиль везде:** декорации окон, панель, dock, launcher и любые элементы `gui_server` используют те же токены темы и DSS, что и приложения (M7.5). Приватных цветов/размеров в `gui_server` не остаётся (сейчас часть геометрии/цветов — через `dwm_settings` config, часть — литералы в `main.rs`; литералы убрать в токены).
- [ ] Оценить вынос оболочки (панель, dock, launcher, меню) в отдельные SDK-клиенты. Сравнить по ИЗМЕРЕНИЯМ: время кадра, задержка ввода, устойчивость (падение клиента оболочки НЕ должно ронять композитор), сложность запуска при загрузке. Вынос включать в план ТОЛЬКО если критерии выполняются; иначе оболочка остаётся в `gui_server`, но использует общую тему. (В протоколе зарезервирован диапазон опкодов DWM-policy `0x1000–0x1fff` под внешнюю оболочку.)
- [x] Добавить baseline-маркер времени кадра перед миграцией `gui_server`: живой compositor печатает `frame ~N ms`, исходный GUI-smoke сохраняет нагрузочную сцену. Требование не ухудшать показатель при будущей миграции остаётся; целевые ≤16 ms и вынос shell не объявляются достигнутыми.

**Критерии приёмки:** `gui_server` компилируется с токенами-темой, грэп = 0 приватных hex/размеров; при выносе оболочки — замеры (кадр/латентность/краш-устойчивость/boot) не хуже baseline, иначе вынос не делается; маркер `frame ~N ms` ≤ baseline. **Риски:** регресс производительности — baseline-маркер обязателен до/после. **DoD:** единый стиль; вынос — только по выполненным критериям.

#### M7.10. C-слой SDK (`dunit.h`) — писать и компилировать C-программы (в т.ч. GUI) ВНУТРИ Dunit

Чтобы программы (включая GUI) можно было писать на C и компилировать прямо в системе через on-device TinyCC (капстоун M6). C-слой — ТОНКАЯ обёртка над тем же Rust UI-рантаймом, НЕ самостоятельный тулкит (иначе второй источник правды).

- [ ] Единый заголовок `<dunit.h>`: обёртки системных вызовов, ошибки (`errno`), файлы, процессы, GUI-клиент. stb-стиль (`#define DUNIT_IMPLEMENTATION`) допустим, если подходит.
- [ ] GUI на C = тонкая C-обёртка над Rust UI-рантаймом (`dunit_ui/dunit_style/dunit_widgets`): статическая библиотека собирается НА ХОСТЕ, кладётся в sysroot (`/usr/lib`), API объявлен в `dunit.h` (`/usr/include`). Одна реализация виджетов, одна тема, один вид и поведение везде. Отдельного C-тулкита не создавать.
- [ ] API колбэчный (в C нет замыканий/дженериков), без дублирования логики Rust SDK. В C-слой НЕ добавлять возможности, которых нет в Rust-рантайме.
- [~] Генерировать C-заголовки из общих описаний. **Готово:** `abi/include/dunit/syscall.h`, `errno.h`, `rights.h` через `tools/gen_abi.py`. **Осталось:** полный `dunit.h`, GUI/DnD types и C ABI UI runtime; ручной второй toolkit не создаётся.
- [ ] Зафиксировать стабильный **C ABI UI-рантайма** (как API формируется) — это входное требование к проектированию рантайма (M7.3/M7.4): экспортируемые C-символы, раскладки структур, версия.

**Предпосылки и риски (явно):**
- `tcc -run` (компиляция в память + немедленное исполнение) требует записать код и сменить права страницы на исполняемую; в текущем ABI уже есть `Mprotect`/RW→RX. Наличие механизма не закрывает executable-memory/JIT policy и поддержку `tcc -run`; новые capabilities/изменения ABI требуют решения человека. Без него остаётся компиляция в файл (`tcc -o a.out` + `posix_spawn`, уже работает в M6).
- Состав/размер в образе: `tcc` (stripped) + заголовки + `libc.a` + GUI-статик-библиотека (напр. `/usr/include`, `/usr/lib`); оценить суммарный размер initrd/`/system` и держать его под контролем.
- Зависит от стабильности M6: запись файлов, простой `posix_spawn` и запуск нового ELF работают; kernel Pipe есть, musl pipe gate требует исправления; process-image `exec` и spawn file-actions не готовы.
- Две поверхности SDK (Rust + C) удваивают поддержку — C-слой держать ТОНКИМ, строго над Rust-рантаймом.

**Цель:** разработка C-программ с GUI целиком внутри Dunit, единый вид с Rust-приложениями. **Зависимости:** M6 (tcc/musl/posix_spawn/pipe), M7.3/M7.4 (рантайм + стабильный C ABI), M7.6 (виджеты), M7.8 (генерация), M7.7 (типы DnD). **Результат:** `<dunit.h>` + GUI-статик-библиотека в sysroot; C-программа с окном компилируется `tcc` на устройстве и работает. **Критерии приёмки (qemu_test):** C-программа с окном (`tcc /assets/.../hello_gui.c -o /tmp/x && exec /tmp/x`) открывает окно через C-обёртку, компоновщик презентует кадр — маркер `[CGUI] ok`; грэп/проверка: syscall/errno/rights/gui-v1-константы в `dunit.h` получены генерацией (регенерация даёт тот же файл, 0 ручных дублей); C-приложение, собранное против `dunit.h`, переживает пересборку рантайма (C ABI стабилен). **Риски:** дрейф C-ABI рантайма; расползание C-слоя в дубль Rust SDK — держать тонким и генерируемым. **DoD:** C GUI-программа компилируется на устройстве и рисует теми же виджетами/темой, что Rust-приложения; заголовки генерируются, не дублируются; самостоятельного C-тулкита нет.

#### M7.11. Миграция программ на SDK (последние пункты M7)

Для КАЖДОЙ программы (Tier 3): маркерный тест проходит на СТАРОЙ версии, затем на НОВОЙ с ТЕМ ЖЕ результатом (для `dufetch` и прочих CLI — тот же вывод); старая версия удаляется ТОЛЬКО после этого.

- [ ] Простейшие первыми: `dufetch` и `wc_demo` (только stdout, вывод легко сверить byte-to-byte).
- [~] Источник фактов для `dufetch`: нынешняя версия читает доступные runtime stats и больше не печатает прежние литералы OS/Kernel/FS, но отдельного versioned uname/sysinfo-контракта и SDK-миграции нет. Новые поля/ABI — по решению человека; недоступные факты не выдумывать.
- [~] `calc` перенесён на native SDK entry/io: ручные `_start`, panic handler, unsafe trim и вывод чисел удалены; вывод и работа через cooked PTY проверяются прежним `dsh_calc_test`. `gui_stat` использует SDK entry/fs/stats/ComponentWindow и typed view; ручная строковая сборка UI и pipeline удалены, стиль вынесен в DSS. Остальные CLI/TUI (`dtop, fsck_dunit, bmp_viewer`), GUI и `dsh` ещё требуют миграции; cargo dunit уже доступен.
- [ ] Системные сервисы `gui_server`/`init` — только на нижние слои SDK (обёртки/рантайм/протокол), без переписывания целиком (M7.9).

**Цель:** приложения реально используют SDK; дубли и магия удалены. **Зависимости:** все предыдущие M7.*; uname-источник (Решение человека) для `dufetch`. **Результат:** Tier-3 программы живут на SDK; старые обёртки удалены. **Критерии приёмки:** для каждой программы её существующий маркер-тест (`tools/*_markers.json`) зелёный на новой версии с тем же выводом; `dufetch` показывает факты из нового источника (не литералы); грэп = 0 дублей магических чисел и ручных gui-v1 handshake в клиентах. **Риски:** «фейковая миграция» (переименование, конфиг, который никто не читает). **DoD (готовность SDK):** прикладные программы РЕАЛЬНО используют SDK, а старые обёртки/дублирующий код/магические числа удалены. **SDK, на который никто не перешёл, считается невыполненным (второй источник правды).**

#### M7.12. Порядок внутри M7 и зависимости

```text
M7.1 wait-set (ABI) -> M7.3 рантайм (A/B) -> M7.2 инструментарий+манифест
 -> M7.4 Component/diff -> M7.5 токены/темы/эффекты -> M7.6 виджеты
 -> M7.8 централизация протокола -> M7.7 DnD/буфер обмена -> M7.9 стиль gui_server
 -> M7.10 C-слой dunit.h (генерация заголовков + C-обёртка рантайма; после M7.6/M7.8)
 -> M7.11 миграция программ (dufetch/wc_demo -> CLI/TUI -> GUI; старое удаляется после паритета)
```

wait-set (M7.1) и выбор рантайма (M7.3) блокируют почти всё и идут первыми (оба — Решения человека). C-слой (M7.10) зависит от стабильного C ABI рантайма (M7.3/M7.4), виджетов (M7.6) и генерации заголовков (M7.8). Миграция (M7.11) — последняя и определяет готовность всего M7. M7 блокирует M8 (wait-set + `dunit::net` + init-манифест), M9 (виджеты/темы + on-device C-разработка) и M10.

### M8 — сеть, платформенные сервисы и экосистема (later; зависит от M7 SDK + wait-set, часть можно параллельно после M1)

#### M8.1. Честное текущее состояние networking

`kernel/src/drivers/net.rs` сейчас выполняет только PCI discovery: распознаёт E1000/RTL8139/legacy VirtIO IDs, для E1000 временно отображает MMIO, читает status и MAC, затем снимает mapping. RX/TX descriptors, packet buffers, interrupts, Ethernet frames и protocol stack отсутствуют; serial log прямо печатает `packet-io=not-implemented` и `stack=not-implemented`. `userspace/system_apps/gui_stat` также корректно показывает `Network (discovery only)`. Поэтому наличие `net0` в registry не считается сетевой поддержкой.

Первый reference target — **E1000 в QEMU**. VirtIO-net добавляется после доказанного end-to-end packet path и modern VirtIO transport; RTL8139 остаётся later/compatibility, а не параллельным отвлечением.

#### M8.2. Целевая архитектура: driver в kernel, stack в `netd`

```text
applications / browser network service / package client
                    |
          native Dunit socket protocol
                    |
             libdunit / musl adapter
                    |
          IPC + readiness event handles
                    |
        netd (userspace network service)
  Ethernet, ARP/NDP, IP, ICMP, UDP, TCP, DHCP, DNS
                    |
      shared bounded RX/TX packet queues
                    |
 Green Tea Kernel NIC driver + IRQ + DMA/IOMMU policy
                    |
              E1000 / VirtIO-net
```

Green Tea Kernel отвечает только за PCI/MMIO, DMA-safe memory, NIC reset/configuration, IRQ moderation, bounded descriptor rings, packet ownership и capability-gated transfer в `netd`. IP, TCP, DNS и policy не должны разрастаться внутри ядра. `netd` — supervised privileged service; приложения не получают MMIO, DMA buffers или raw Ethernet по умолчанию.

- [ ] Добавить `NET_DEVICE`/`PACKET_QUEUE` handles с rights `RX`, `TX`, `CONTROL`, `RAW`, `TRANSFER`.
- [ ] Передавать пакеты между driver и `netd` через bounded shared rings и event handles, без syscall на каждый byte.
- [ ] Зафиксировать ownership state каждого buffer: `DRIVER_RX -> NETD -> FREE` и `APP/NETD_TX -> DRIVER -> COMPLETE`.
- [ ] При падении `netd` kernel останавливает queue, отзывает mappings, сбрасывает NIC и позволяет service manager запустить его заново.
- [ ] Raw packet capability выдавать только diagnostics/network services; обычное приложение получает socket endpoint с ограниченными правами.
- [ ] Конфигурацию интерфейсов и policy хранить вне kernel; driver не знает DHCP, DNS, routes или proxy.

**Цель:** модульный native stack без Linux network ABI. **Причина:** TCP/DNS в kernel усложнят recovery, безопасность и независимое обновление. **Подсистемы:** PCI, VMM/DMA, interrupts, IPC/shared memory, handles, init, `netd`. **Зависимости:** M1 handles/events/shared VM и service supervision; **M7 SDK** (безопасные обёртки `dunit::net`, event loop) и **wait-set** из M7 (единый `event.wait` над socket+IPC+timer+file — см. M8.7); init-манифест для `netd` (как `gui-server.toml`). **Результат:** перезапускаемый userspace stack над узким packet I/O backend. **Готовность:** crash/restart `netd` не повреждает kernel и возвращает интерфейс в рабочее состояние. **Риски:** копирование и context-switch overhead; решать shared rings/batching, не переносом всего stack в kernel.

#### M8.3. N0 — E1000 packet I/O foundation

- [ ] Реализовать hardware reset, EEPROM/RAL-MAC selection, link state и documented register initialization.
- [ ] Выделять физически пригодные DMA regions через общий DMA API; не использовать произвольные virtual pointers как bus addresses.
- [ ] Реализовать RX/TX descriptor rings, head/tail management, buffer recycling и memory barriers.
- [ ] Настроить MSI/MSI-X при наличии, с fallback на legacy interrupt; polling разрешён только как диагностический режим.
- [ ] Добавить interrupt moderation/NAPI-like bounded drain без копирования Linux API.
- [ ] Обработать link up/down, queue stall, malformed/oversized frames, ring exhaustion и reset recovery.
- [ ] Публиковать counters: packets/bytes, drops by reason, checksum errors, queue full, resets, link transitions.

**Результат:** driver отправляет и принимает реальные Ethernet frames. **Готовность:** двунаправленный frame loop через QEMU network backend; 10-minute flood не зависает и не течёт. **Тесты:** ring wrap, RX starvation, TX completion, link toggle, invalid length, interrupt storm, forced reset. **Риски:** DMA corruption и races; guard regions, ownership assertions и bounded descriptors обязательны.

#### M8.4. N1 — link/network layers: Ethernet, ARP, IPv4 и ICMP

- [ ] Ethernet II parser/builder: destination/source MAC, EtherType, minimum/maximum frame size и padding.
- [ ] Drop неизвестных/повреждённых frames до глубокого parsing; каждый length проверять до чтения header.
- [ ] ARP cache со states `INCOMPLETE/REACHABLE/STALE`, bounded entries, retry/expiry и защита от бесконтрольного poisoning.
- [ ] IPv4 validation: version/IHL, total length, header checksum, TTL, protocol, source/destination и interface ownership.
- [ ] Реализовать routing table с connected/default routes и longest-prefix match; не зашивать один gateway в код.
- [ ] Зафиксировать fragmentation policy: v1 может не фрагментировать TX, но обязан корректно вернуть MTU error; RX reassembly — bounded по bytes/fragments/time либо явно later.
- [ ] ICMPv4 echo request/reply и необходимые destination-unreachable/time-exceeded сообщения; rate limiting обязателен.
- [ ] Реализовать loopback как логический interface без обращения к физическому NIC.

**Результат:** статически настроенный Dunit host отвечает на ping и пингует gateway. **Готовность:** ARP resolution + ICMP echo проходят при cache miss/hit/expiry; malformed packets не валят `netd`. **Тесты:** bad checksum/length/IHL, TTL zero, unknown EtherType/protocol, ARP timeout, route miss, MTU boundary, packet fuzzing. **Риски:** parser vulnerabilities и unbounded reassembly; использовать checked cursor API и жёсткие budgets.

#### M8.5. N2 — UDP, DHCPv4 и DNS resolver

- [ ] UDP demultiplexing по local address/port, ephemeral port allocation, checksum и bounded receive queues.
- [ ] Native datagram endpoint: bind/connect/send-to/receive-from, nonblocking mode, timeout и readiness events.
- [ ] DHCPv4 state machine `INIT -> SELECTING -> REQUESTING -> BOUND -> RENEWING/REBINDING`, lease timers и link-change reset.
- [ ] Валидировать DHCP offers/options; применять address, prefix, gateway и DNS atomically, с rollback на ошибке.
- [ ] Сохранять lease только как optimization; после reboot/link change проверять его заново и не считать вечным.
- [ ] DNS stub resolver: A/AAAA/CNAME, UDP query IDs, compression-pointer bounds, retry/timeout, bounded TTL cache и TCP fallback для truncated replies.
- [ ] Источники конфигурации: static system profile, DHCP и user/admin override с явно определённым precedence.

**Результат:** интерфейс получает адрес автоматически, разрешает имя и обменивается UDP datagrams. **Готовность:** DNS lookup не требует hardcoded server; lease renew и timeout корректны. **Тесты:** DHCP loss/NAK/renew, duplicate address response, DNS malformed compression, NXDOMAIN, timeout/retry/cache expiry, UDP queue overflow. **Риски:** spoofing и parser loops; transaction validation, budgets и privileges.

#### M8.6. N3 — TCP transport

- [ ] Реализовать отдельную TCP state machine для active/passive open и states от `CLOSED` до `TIME_WAIT`.
- [ ] Проверять 4-tuple, sequence/acknowledgement numbers, flags, header length, checksum и receive window.
- [ ] Реализовать SYN retransmission, data retransmission, RTT/RTO estimator, duplicate ACK handling и bounded retry policy.
- [ ] Добавить sliding send/receive windows, out-of-order queue с memory limit, FIN/RST handling и half-close.
- [ ] Начать с простого проверяемого congestion control (например, Reno-подобного); congestion control не может отсутствовать в Internet-facing release.
- [ ] Поддержать listen backlog, accept queue, ephemeral ports, `SO_REUSEADDR`, keepalive policy и `TCP_NODELAY`; редкие options later.
- [ ] Все timers вести от monotonic clock и обрабатывать wrap/large jumps детерминированно.

**Результат:** надёжный byte stream для локальных и внешних соединений. **Готовность:** клиент и сервер передают multi-megabyte stream с искусственными loss/reorder/duplication; connection teardown не течёт. **Тесты:** handshake timeout, simultaneous close, reset, zero window, retransmission, reordered segments, TIME_WAIT pressure, SYN/backlog exhaustion. **Риски:** сложность TCP и resource exhaustion; state/property tests, per-socket/global quotas и сначала один congestion algorithm.

#### M8.7. N4 — native Dunit socket API и event model

Native API является service protocol/object model, а не копией Linux syscall table:

```text
net.open(family, type, protocol) -> socket handle
net.bind(handle, endpoint)
net.connect(handle, endpoint)
net.listen(handle, backlog)
net.accept(handle) -> socket handle + peer
net.send/receive(handle, buffers, flags)
net.send_to/receive_from(handle, endpoint, buffers, flags)
net.shutdown(handle, direction)
net.get/set_option(handle, typed option)
resolver.lookup(name, family, flags) -> address list
event.wait([socket readiness, IPC, timer, file], deadline)
```

- [ ] Версионировать socket/resolver protocol независимо от TCP implementation.
- [ ] Представлять IPv4/IPv6 endpoints typed structures, не строками внутри protocol.
- [ ] Определить partial I/O, EOF, reset, timeout, would-block и cancellation semantics.
- [ ] Интегрировать readiness в общий Dunit event wait; не добавлять busy polling в GUI/browser.
- [ ] Разделить права `CONNECT`, `BIND_LOW_PORT`, `LISTEN`, `RAW`, `CONFIGURE_INTERFACE` и `OBSERVE_ALL`.
- [ ] Добавить per-process quotas на sockets, queued bytes, DNS requests и listening backlog.
- [ ] Поддержать capability revocation и deterministic errors при рестарте `netd`.

**Результат:** Rust/Dunit-native приложения используют сеть без POSIX и без доступа к NIC. **Готовность:** echo client/server работают через public `libdunit` API; один зависший клиент не блокирует `netd`. **Тесты:** readiness edge cases, close during wait, partial send, quota denial, handle transfer, service restart. **Риски:** premature ABI freeze; сначала versioned protocol + conformance suite.

#### M8.8. N5 — подключение к Dunit musl

```text
POSIX C API (`socket`, `connect`, `send`, `poll`, `getaddrinfo`)
        -> Dunit musl adapter
        -> libdunit native socket/resolver protocol
        -> netd
```

- [ ] Реализовать musl mappings для `socket/bind/connect/listen/accept`, send/receive family, `shutdown` и close.
- [ ] Переводить POSIX `sockaddr`, flags/options и errors в versioned Dunit types; kernel и `netd` не принимают Linux structs.
- [ ] Реализовать blocking/nonblocking behaviour поверх readiness events, включая timeout и interruption policy.
- [ ] Подключить `poll`/`pselect`-подобный libc слой к общему Dunit event wait, чтобы socket, pipe, IPC и timer ожидались вместе.
- [ ] Перенести `getaddrinfo/getnameinfo` на resolver protocol; не заставлять каждое приложение самостоятельно разбирать DNS.
- [ ] Документировать supported socket options и возвращать `ENOPROTOOPT` для неподдерживаемых, а не fake success.

**Результат:** одна и та же native сеть обслуживает Dunit-native и C/POSIX applications. **Зависимости:** M6 static runtime + N4 API. **Готовность:** статические C echo/DNS/HTTP clients проходят без Linux ABI. **Тесты:** POSIX error mapping, IPv4/IPv6 address conversion, nonblocking connect, poll timeout/readiness, resolver concurrency. **Риски:** расхождение POSIX и native semantics; contract tests запускаются против обоих APIs.

#### M8.9. N6 — TLS, HTTP и путь к GUI-браузеру

TLS и HTTP не помещаются в kernel или NIC driver. Они работают в отдельном userspace network service браузера либо в общей high-level библиотеке:

```text
browser-ui (GUI client)
    |
browser content processes
    |
browser-network-service
  URL policy, proxy, cache, cookies, HTTP, TLS, certificates
    |
musl sockets или native Dunit socket API
    |
netd -> NIC driver
```

- [ ] До TLS предоставить cryptographically secure entropy, корректное realtime clock и обновляемое read-only certificate store.
- [ ] Портировать проверенную TLS/crypto library; не писать собственную криптографию как часть network stack.
- [ ] Ввести trust-store update/version/rollback и hostname/certificate validation tests.
- [ ] Реализовать HTTP/1.1 client с redirects, chunked/content-length framing, compression limits и connection reuse; HTTP/2 позже поверх доказанного TLS/TCP.
- [ ] Вынести browser networking в отдельный sandboxed process; renderer/content process не получает unrestricted sockets.
- [ ] Network service применяет origin/proxy/download policy, quotas и передаёт браузеру responses через IPC/shared buffers.
- [ ] Persistent browser cache/cookies живут в user data directories и используют size limits/atomic updates; это не состояние `netd`.
- [ ] GUI Browser является обычным GUI client: создаёт surfaces через GUI Server и не получает framebuffer/input/network master capabilities.
- [ ] Сначала сделать headless HTTP fetch + minimal GUI response viewer; полноценный HTML/CSS/JS browser engine — отдельный поздний проект.

**Результат:** архитектура сети изначально пригодна для браузера, но browser engine не блокирует network stack. **Готовность первого browser-network slice:** GUI-приложение просит sandboxed network service загрузить HTTPS resource, получает body/status через IPC и отображает результат через GUI Server. **Тесты:** invalid/expired/wrong-host certificate, redirect loop, truncated/chunked response, decompression bomb limit, cache corruption, network-service crash/restart. **Риски:** огромный security surface; process isolation, established crypto/TLS, strict parsers и no ambient network access.

#### M8.10. N7 — IPv6 и multi-interface (после стабильного IPv4 vertical slice)

- [ ] Ethernet multicast, IPv6 header validation, ICMPv6, Neighbor Discovery и Duplicate Address Detection.
- [ ] Link-local addresses, SLAAC, router advertisements и DNS configuration; DHCPv6 только по подтверждённой необходимости.
- [ ] IPv6 UDP/TCP через тот же transport/socket API, без отдельного application path.
- [ ] Несколько interfaces, per-interface addresses/routes/DNS, route metrics и link failover.
- [ ] Happy Eyeballs policy реализовать в resolver/client layer после стабильного dual-stack.

**Готовность:** IPv4-only, IPv6-only и dual-stack tests используют одни application APIs. **Риски:** преждевременное удвоение scope; packet/address abstractions проектируются dual-stack заранее, реализация IPv6 следует после N1–N6.

#### M8.11. Network configuration, observability и security

Предлагаемое состояние:

```text
/system/share/network/defaults.toml       immutable defaults
/system/services/netd.toml                service manifest/capabilities
/users/<uid>/config/network/              allowed user preferences
/var/lib/netd/                            leases and bounded service state
/var/log/netd/                            bounded/rotated diagnostics
/run/netd/                                volatile endpoints/status
```

- [ ] Schema-versioned static/DHCP configuration с validate -> stage -> atomic apply -> rollback.
- [ ] Status API: interfaces, addresses, routes, DNS, link state и counters без раскрытия чужого traffic.
- [ ] Diagnostic tools: `ip`-подобный status, route, ping, DNS lookup, TCP connect и privileged bounded packet capture.
- [ ] Default deny для raw sockets, interface configuration и low ports; application manifests запрашивают network scopes.
- [ ] Ограничить packet sizes, fragment queues, socket buffers, DNS cache, TCP states и log volume.
- [ ] Все parsers fuzz/property-test; входной packet никогда не приводит к panic всего `netd`, kernel или desktop.
- [ ] Не заявлять firewall/VPN как готовые до появления routing/hooks/key storage; предусмотреть policy hooks без ранней реализации сложного framework.

#### M8.12. Network milestone order и acceptance matrix

```text
N0 E1000 RX/TX
 -> N1 Ethernet/ARP/IPv4/ICMP
 -> N2 UDP/DHCP/DNS
 -> N3 TCP
 -> N4 native sockets/events
 -> N5 musl sockets
 -> N6 TLS/HTTP/browser-network slice
 -> N7 IPv6/multi-interface
```

N4 API проектируется параллельно N1–N3, но фиксируется только после работающих semantics. N5 musl adapter и N6 HTTP tests можно разрабатывать против host mock/reference server, затем переключить на `netd` conformance endpoint.

Обязательная автоматическая матрица через `tools/qemu_test.py`:

- QEMU E1000: link/reset, ARP, ping, UDP, DHCP, DNS, TCP client/server;
- deterministic isolated test network, без зависимости CI от публичного Internet;
- loss/reorder/duplicate/corruption/latency injection;
- malformed packet corpus и bounded fuzz runs;
- `netd` crash/restart и NIC reset во время активных sockets;
- static musl C socket client рядом с native Rust client;
- HTTPS test против локального deterministic server и тестового trust root;
- GUI response viewer через GUI Server без прямых network/display privileges;
- soak: множество connect/close, DNS cache expiry, TCP retransmission и memory/handle leak accounting.

**Критерий завершения networking v1:** E1000 выполняет реальный packet I/O; `netd` получает DHCP address, резолвит DNS и держит TCP stream; native и static-musl clients проходят одинаковые conformance cases; локальный HTTPS resource загружается через sandboxed browser-network service; reboot сохраняет только разрешённую configuration/state; malformed traffic и падение `netd` не рушат kernel/GUI. Публичный Internet smoke допустим как optional manual/release test, но не заменяет deterministic CI.

#### M8.13. Остальные platform services

- [ ] Audio: HDA discovery/DMA -> mixer service -> per-app streams/permissions.
- [ ] USB: xHCI enumeration -> descriptors/endpoints -> HID keyboard/mouse -> mass storage; hotplug events.
- [~] Power: reboot/poweroff работают и имеют `tools/m6_reboot_markers.json`, `tools/m6_poweroff_markers.json`; ACPI battery/thermal и suspend/resume не готовы.
- [ ] Packages: signed bundle manifest, architecture/ABI version, install transaction, rollback; network repository только после network + trust model. Пакеты строятся инструментом M7 SDK (`cargo dunit`), а app-manifest — это `dunit.toml` из M7.

**Цель/причина:** превратить desktop prototype в платформу. **Зависимости:** M1, M5, ABI, M7 SDK (app-model/manifest/package builder); packages из сети зависят от crypto/network/time (M8.9). **Готовность:** subsystem-specific end-to-end tests и permission denial tests. **Риски:** hardware matrix; QEMU reference devices first, затем реальные machines.

### M9 — удобные программы на SDK (после M7; GUI-часть после M7.6)

Набор полезных программ, построенных НА SDK (в отличие от M7.11, который мигрирует существующие). Флагман — собственный аналог nano (рабочее название **rock**). Все — через `cargo dunit`, на виджетах/темах SDK, без ручной отрисовки и магических чисел.

Предлагаемый состав:
- [ ] **rock** — текстовый редактор уровня nano: открытие/правка/сохранение, курсор/выделение, поиск/замена, undo/redo, статус-строка, горячие клавиши. Сначала TUI (в `gui_terminal`/консоли), затем GUI-обёртка на тех же буферах.
- [ ] **GUI-редактор текста** поверх ядра `rock` (multiline-поле ввода M7.6, прокрутка, темы).
- [ ] **Просмотрщик изображений** на SDK (заменяет raw-framebuffer `bmp_viewer`): открытие из `gui_files`, зум/вписать, декодеры из общего набора (BMP есть; PNG/JPEG — задел для M10).
- [ ] **Просмотрщик текста/Markdown** (read-only, типографика по токенам).
- [~] **Полировка системных GUI-приложений**: терминальный setup, графические Settings/presets, исправления pointer/mode-switch, Lua tiling, SVG/Chameleon и режимы до 3K уже есть. Миграция на Component/widgets SDK, системный clipboard/DnD, полное поведение widgets и acceptance M7.11 не завершены.

**Сценарий разработки ВНУТРИ системы (замыкает M6 tcc + M7 C-слой):** редактор `rock` → компиляция `tcc` → запуск. Пользователь пишет C-программу (в т.ч. с окном через `<dunit.h>`, M7.10) в `rock`, компилирует её on-device (`tcc hello_gui.c -o /tmp/x`) и запускает (`exec /tmp/x`) — без хоста.

- [ ] Довести путь rock→tcc→exec до рабочего на устройстве (редактор сохраняет файл, tcc компилирует с заголовками/библиотеками из sysroot, `posix_spawn`/exec запускает).

**Цель:** набор повседневных приложений + полный on-device цикл разработки, доказывающие, что на SDK (Rust и C) удобно и красиво писать. **Зависимости:** M7 (виджеты M7.6, темы M7.5, инструментарий M7.2, C-слой M7.10); on-device цикл ← M6 (tcc, запись файлов, posix_spawn); просмотрщик изображений ← декодеры (общие с M10). **Результат:** пользователь редактирует файлы, смотрит картинки/текст в едином стиле и может написать+собрать+запустить C-программу с окном не выходя из системы. **Критерии готовности (per-app):** собирается и запускается только через `cargo dunit`; использует SDK-виджеты (грэп = 0 ручного `fill_rect`/hit-test в прикладном коде); полностью управляем с клавиатуры (Tab-обход, мнемоники); проходит marker-тест функций (rock: открыть→правка→сохранить→перечитать тот же хеш; редактор: ввод/undo/redo/поиск; просмотрщик: открыть файл известного размера, маркер `[ROCK] ok`/`[IMGVIEW] ok`); визуальный эталон (M7.5) в обеих темах. **Критерий приёмки on-device-цикла (qemu_test):** тестовая C-программа с окном, написанная на C, компилируется `tcc` ВНУТРИ QEMU и запускается — проверка маркерами (`[CDEV] compiled+ran ok` + окно презентовано). **Риски:** расползание набора — фиксировать состав, не добавлять «на вкус». **DoD:** rock + минимум просмотрщик изображений и редактор реально используются, raw-framebuffer `bmp_viewer` удалён после паритета; on-device rock→tcc→run цикл зелёный.

### M10 — браузер (поздний; крупнейший риск-пункт)

GUI-браузер как обычный клиент GUI Server (создаёт surfaces, НЕ получает framebuffer/input/network master). Это самый крупный и наименее оцениваемый пункт — здесь честно перечислены зависимости, варианты объёма и риски; обещаний того, что нельзя оценить, нет.

**ПЕРВЫЙ ШАГ (с пользователем):** продумать движок — **портировать существующий** (напр. облегчённый HTML/CSS-движок) **или писать с нуля**. Это решение принимается с пользователем, КОГДА он дойдёт до M10 по роадмапу (агент спрашивает тогда, а не фиксирует выбор здесь).

Зависимости (всё должно существовать до реального рендера страниц):
- сеть: `netd` + TCP + DNS (M8.6–M8.8) и sandboxed browser-network-service (M8.9);
- TLS: безопасная энтропия + корректные realtime-часы (нужен RTC — сейчас `CLOCK_REALTIME`==monotonic) + обновляемый trust store + портированная проверенная TLS-библиотека (M8.9);
- шрифты: шейпинг/кёрнинг/перенос строк и сложные скрипты (сейчас `dunit_text` — только простые advance'ы, одна строка);
- декодеры изображений: PNG/JPEG/GIF/WebP (сейчас только BMP/`.rgba`);
- движок разметки/стилей/скриптов: HTML-парсер, CSS cascade+layout, JS-движок — КАЖДЫЙ сам по себе крупный проект.

Реалистичные варианты объёма (выбор — с пользователем на первом шаге):
- **Viewer-slice (минимум):** headless HTTPS fetch (есть в M8.9) + отрисовка упрощённого HTML/текста без JS. Оцениваемо, строится на M8.9.
- **Облегчённый порт движка:** взять существующий малый HTML/CSS-движок, портировать на SDK/музл — риск по размеру/зависимостям, но без написания layout/JS с нуля.
- **С нуля:** полный свой движок — крупнейший объём и риск; оправдан только явным решением человека.

**Цель:** архитектура изначально пригодна для браузера, но движок не блокирует сеть/SDK. **Зависимости:** M7 (SDK/виджеты), M8 (сеть/TLS/HTTP), M9-уровень шрифтов/декодеров. **Результат (первого slice):** GUI-приложение просит sandboxed network-service загрузить HTTPS-ресурс, получает body/status через IPC и отображает результат через GUI Server. **Критерии приёмки:** первый slice — локальный детерминированный HTTPS-сервер + тестовый trust root, страница грузится и отображается через GUI Server без прямых network/display-привилегий; парсеры fuzz/property-тестируются, битый ввод не роняет сервис/десктоп. **Риски:** гигантский security + объёмный surface; строгие парсеры, process isolation, проверенная крипта, никакого ambient-доступа к сети. **DoD:** определяется выбранным на первом шаге вариантом объёма — зафиксировать критерий вместе с выбором движка.

## 15. План полной переработки GUI Mode (историческое — выполнено)

Практическая последовательность — M2 protocol model, затем один M3 vertical slice «два клиента -> shared surfaces -> compositor -> input/focus», после него M4 parity. Старый GUI остаётся feature-gated только до появления terminal + panel + dock + launcher + notifications + quick settings. После parity удаляются `kernel/src/ui_loop.rs`, `kernel/src/window_manager.rs` и GUI-specific syscall drawing; reusable framebuffer/damage primitives переносятся в userspace либо остаются узким backend API.

## 16. План конфигурационной UI-системы

Три схемы (`config.toml`, `*.dui`, `*.dss`) получают независимые version fields, size/depth limits и offline linter. DWM применяет конфигурацию транзакционно: parse -> schema validate -> resource resolve -> layout dry run -> atomic swap. Ошибка сохраняет last-known-good, пишет diagnostic и никогда не валит desktop. System defaults неизменяемы; user overlay хранится на persistent user volume.

## 17. План перехода от Live Boot к установленной системе

M5 заменяет embedded `/app` и assets на disk-root manifest, запускает `init` с диска и чётко различает profiles: Live монтирует read-only image и предлагает installer/recovery, Installed монтирует persistent system/data и не требует installer modules. Installation success означает не запись ESP, а успешный first boot, создание user environment, запись данных и повторную проверку после reboot.

## 18. План persistence и файловой структуры

DunitFS v2 должна иметь recoverable metadata update, `fsync` contract, allocation tracking и `fsck`. System defaults лежат в `/system/share`; изменяемые host state — `/var`; user settings/data — `/users/<uid>/{config,data,home}`; `/run` всегда volatile. Acceptance test записывает random content и config, синхронизирует, полностью останавливает VM, загружает тот же image и сверяет hashes и DWM settings.

## 19. План портирования libc

M6 фиксирует полный musl fork как основной путь, Dunit-native syscall adapter и static-first порядок. Работа начинается не с `printf`, а с versioned userspace ABI, process-entry contract, воспроизводимого sysroot и custom `crt1`. Затем последовательно вводятся bootstrap, VM/allocator, filesystem, spawn и pthread/TLS. Критический kernel path: `mmap/munmap/mprotect -> preemption -> thread lifecycle -> FS.base/TLS -> atomic wait/wake -> pthread subset`.

Практический первый deliverable — команда `x86_64-dunit-cc -static hello.c -o hello`, создающая ELF без host/Linux dependencies. Финальный deliverable M6 — документированный static libc profile с files, process spawn и pthread subset. Signals, `fork` и dynamic loader не входят в первый порт; их семантика проектируется только по подтверждённому demand. POSIX headers/API являются source-compatibility facade над Dunit ABI.

Upstream-ориентиры для реализации и проверки assumptions:

- [musl supported platforms](https://wiki.musl-libc.org/supported-platforms) — подтверждает ориентацию upstream на Linux syscall layer;
- [musl porting notes](https://wiki.musl-libc.org/porting) — архитектурные porting resources и рекомендация `libc-test`;
- [musl getting started](https://wiki.musl-libc.org/getting-started) — static-only build через `--disable-shared` как штатный режим.

Эти материалы не задают архитектуру Dunit: Dunit-specific OS layer, ABI profiles, process model и kernel primitives определяются данным roadmap и отдельной ABI-спецификацией.

## 20. Критерии завершения каждого этапа

- **M0:** status claims трассируются к test marker и исходнику.
- **M1:** preemption/thread/TLS/wait-wake stress проходит без races и leaks.
- **M2:** protocol conformance/fuzz suite детерминированно зелёная.
- **M3:** userspace GUI Server композитит два изолированных клиента и переживает их crash.
- **M4:** feature parity и config reload без kernel GUI code.
- **M5:** BIOS/UEFI installed root + content/config persistence + recovery test.
- **M6:** static C hello/file/malloc/thread suite на Dunit musl fork.
- **M8 networking v1:** E1000 packet I/O -> `netd` DHCP/DNS/TCP -> native sockets -> static-musl client -> local HTTPS browser-network slice проходит end-to-end; остальные services имеют собственные I/O и permission gates, не только discovery.

## 21. Dunit Minimal и Dunit DWM images

- **Dunit Minimal:** kernel, init, recovery terminal, storage/fs tools, network diagnostics optional; без GUI assets/server/DWM. Используется CI, recovery и low-resource systems.
- **Dunit DWM:** Minimal + GUI Server, Dunit UI Runtime, DWM, fonts/themes, terminal/file manager/settings.
- Оба образа используют один kernel ABI и package format; различаются manifests, не `#ifdef` архитектурой.
- Live images дополнительно содержат installer и read-only recovery assets; installed images используют persistent root.

## 22. Зависимости между этапами и параллельная разработка

```text
M0 baseline
 |- M1 scheduler/threads/IPC/VM -----> M3 GUI Server -----> M4 DWM/UI
 |                    `--------------> M6 musl static -> pthread -> dynamic later
 |- M2 protocol/headless tests ------> M3
 `- M5 filesystem/installer ---------> M4 config persistence + M6 disk userspace

M6 (musl/tcc) + M4 (UI runtime) --> M7 SDK (wait-set -> рантайм -> виджеты/темы -> DnD -> миграция)
M7 SDK --> M8 netd/сеть/сервисы (wait-set + dunit::net + init-манифест)
M7 SDK --> M9 программы (rock и др.; GUI после виджетов M7.6)
M7 + M8 + шрифты/декодеры --> M10 браузер
```

Можно параллельно:

- GUI protocol/headless compositor tests и kernel scheduler work;
- DunitFS v2 design/fsck tooling и shared-memory/IPC work;
- DUI/DSS parser/layout host tests и GUI Server integration;
- musl source audit/toolchain spike и M1 implementation;
- network/audio/USB design после handle/event model, не раньше.

Нельзя параллелить ценой фиксации временного ABI: GUI shared buffers, pthread/TLS и package manifests должны ждать утверждённых kernel object/ABI contracts.

## 23. Приоритеты: must-have / should-have / later

**Must-have:** M0; preemption/threads/wait queues/handles/shared VM; GUI protocol/server; DWM parity; DunitFS recovery + disk-root/init; persistence matrix.

**Should-have:** static musl port (сделано), **M7 Dunit SDK** (wait-set, рантайм, декларативный GUI/виджеты/темы, DnD, миграция программ), config hot reload, workspaces/session restore, VirtIO modern block, ACPI shutdown.

**Later:** M8 сеть (`netd`/TCP/TLS), M9 программы на SDK (rock и др.), M10 браузер; dynamic loader, full signals/fork, SMP, GPU acceleration, audio mixer, USB mass storage, signed repositories, suspend/resume.

## 24. Risk register

| Риск | Вероятность/влияние | Мера |
|---|---|---|
| Preemption вскрывает `static mut` races | высокая/критическое | lock audit, IRQ-safe rules, stress before default-on |
| DunitFS corruption | высокая/критическое | v2 metadata transaction, fsck, power-cut tests, backup superblock |
| GUI protocol premature freeze | средняя/высокое | minimal v1, capability negotiation, headless conformance suite |
| Shared buffer leaks/aliasing | высокая/высокое | kernel-owned VM objects, reference counts, rights and bounds |
| GUI rewrite stalls visible progress | средняя/высокое | legacy GUI feature until parity; vertical slices |
| musl drags kernel toward Linux | средняя/высокое | Dunit ABI adapter, documented POSIX layer, no Linux syscall numbers |
| fork/signals consume roadmap | высокая/среднее | static + spawn first; demand-driven semantics |
| Installer destroys wrong disk | низкая/критическое | device identity, size/model confirmation, dry-run plan, explicit destructive token |
| Hardware scope explosion | высокая/среднее | reference QEMU devices and published support tiers |

## 25. Technical debt register

- [ ] Разбить крупные файлы по ответственностям: `kernel/src/lib.rs`, `syscall/mod.rs`, `process/mod.rs`; и `userspace/system_apps/gui_server/src/main.rs` (5143 строки — монолит компоновщика+оболочки, цель M7.9). (`ui_loop.rs` удалён.)
- [~] Устранить `static mut` references и compiler warnings. **Готово:** owned/locked/atomic kernel state и CI marker budget. **Осталось:** предупреждения текущей сборки (включая kernel/libdunit/test apps); наличие CI не означает ноль warnings.
- [ ] Заменить raw VFS trait pointers/global path buffer безопасной ownership/locking model.
- [x] initrd: реальный `DUNITRD1` archive loader (`kernel/src/initrd.rs`) наполняет корневой MemFS; honest boot logs. (Было: пустой объект.)
- [~] Унифицировать Terminal и GUI shell через userspace session/PTY, а kernel оставить recovery console. **Готово:** GUI-терминал исполняет команды через userspace-шелл `dsh` поверх kernel PTY-endpoint. **Осталось:** свести kernel terminal-mode строго к recovery-консоли (сейчас отдельный boot-путь в ядре).
- [~] Версионировать Dunit ABI, filesystem, GUI protocol, app manifest и config schemas. **Готово:** Dunit Userspace ABI v0 (`abi/README.md`) и GUI protocol v1 (M2, `protocols/gui-v1`); config schema_version (dwm_settings). **Осталось:** версионирование filesystem, app manifest (`dunit.toml` — M7) и остальных config schemas.
- [x] Удалены dead `display_server`/`video_driver` крейты (полезные pure-structure тесты перенесены в `protocols/gui-v1/tests/wm.rs`).
- [ ] Централизовать дублированные протокольные магики `CAP1/INP1/RLD1/NOT1` (в 8 файлах) в один источник/генерацию (M7.8).
- [ ] Добавить structured log levels и bounded persistent logs.

## 26. План тестирования

Каждый runtime test запускается только через `tools/qemu_test.py`.

Матрица:

- boot: ISO/disk × Terminal/GUI × BIOS/UEFI;
- storage: AHCI/VirtIO × clean/corrupt/full × two-reboot/power-cut;
- process: spawn/wait/kill/fault/preempt/thread/TLS;
- IPC: queue full, client crash, endpoint close, rights transfer, shared buffer lifecycle;
- GUI: headless protocol suite + QEMU screenshot markers + interactive event injection;
- libc: static C conformance tiers; dynamic tier отдельно;
- network: E1000/link -> ARP/ICMP -> DHCP/DNS/UDP/TCP -> native/musl sockets -> local TLS/HTTP; loss/reorder/malformed traffic и `netd` restart;
- browser-network: sandbox/capability denial, test trust root, certificate/redirect/framing/cache failures, GUI response viewer через GUI Server;
- longevity: 10 min smoke per PR, multi-hour nightly stress.

Обязательные gates milestone:

- serial log не содержит `PANIC`, `FAIL`, unexpected fault или timeout;
- marker доказывает не только boot, но и subsystem completion;
- screenshot используется только вместе с machine-readable state/serial marker;
- persistence считается доказанной лишь после stop/start, повторного mount и проверки content hash;
- corrupt input tests не требуют участия пользователя и QEMU всегда завершается harness-ом.

## 27. Что нельзя делать слишком рано

- не переносить dock/widgets/styles в kernel;
- не писать прикладные программы в обход SDK, когда он есть (второй источник правды — M7.11 DoD);
- не стабилизировать immediate-mode `DRAW_RECT/DRAW_TEXT` как долгосрочный GUI ABI (использовать surface/buffer protocol);
- не давать приложениям raw framebuffer/input;
- не делать полный CSS/HTML/JS runtime до базового DUI/DSS;
- не начинать dynamic loader до stable VM/ABI/filesystem;
- не делать `fork` главным условием musl v1;
- не считать `/persist` эквивалентом installed root;
- не добавлять package network repository без signatures, time, TLS/crypto и transactions;
- не начинать SMP до корректного UP preemption и lock audit;
- не заявлять hardware support по PCI detection без end-to-end I/O test.

## 28. Предлагаемая структура каталогов

```text
kernel/
  arch/x86_64/  memory/  process/  ipc/  vfs/  drivers/  abi/
userspace/
  init/  services/gui-server/  services/inputd/  services/netd/  services/audiod/
  services/browser-network/  dwm/  ui-runtime/  shell/  terminal/  apps/  libdunit/
protocols/
  gui-v1/  net-v1/  resolver-v1/  service-v1/  package-v1/
filesystems/dunitfs/
toolchains/
  dunit-musl/  sysroot/  wrappers/  cmake/  meson/
images/minimal/  images/dwm/  images/live/
configs/defaults/
tests/qemu/  tests/protocol/  tests/persistence/  tests/libc/
tools/qemu_test.py
```

Полное дерево musl живёт в отдельном fork-репозитории `dunit-musl`; `toolchains/dunit-musl/` фиксирует конкретный commit этого fork. Сгенерированный sysroot не хранится как непрозрачный binary blob: он воспроизводимо собирается из pinned compiler, musl source и Dunit ABI headers.

## 29. Дальнейшее развитие проекта

После M7/M8: стабилизировать SDK и ABI release cadence, добавить signed app bundles, accessibility/text shaping/IME, hardware compatibility tiers, reproducible images и upgrade rollback. GPU acceleration, SMP, dynamic linking, suspend/resume и broad POSIX coverage развиваются независимо и не должны блокировать надёжную UP software-composited desktop v1.

## 30. Итоговая целевая архитектура Dunit Desktop OS

Завершённая архитектура — не Linux distribution и не GUI внутри kernel. Green Tea Kernel предоставляет собственные Dunit objects, ABI, scheduling, VM, IPC, VFS и device primitives. `init` запускает userspace services. GUI Server единолично владеет display/input handles и композитит shared surfaces. Независимый Dunit DWM задаёт desktop policy через Dunit UI Runtime и versioned DUI/DSS/TOML. Kernel NIC drivers отдают bounded packet queues отдельному `netd`; native и musl socket APIs сходятся на одном versioned network protocol. Будущий browser использует sandboxed browser-network service для TLS/HTTP и остаётся обычным GUI client без raw display/network privileges. Установленная система грузит programs/config/data с проверяемой DunitFS, а Dunit musl fork предоставляет source portability поверх Dunit ABI.

Ближайший правильный порядок: **M0–M6 выполнены** (honest baseline, preemption/threads/blocking IPC/shared VM, GUI Server + DWM parity, DunitFS/install, static-first musl + on-device tcc). Дальше: **M7 Dunit SDK** (wait-set → рантайм → инструментарий → декларативный GUI/виджеты/темы → DnD → миграция программ), затем **M8 сеть** (`netd`/TCP/DNS/TLS поверх SDK+wait-set), **M9 программы на SDK** (rock и др.) и **M10 браузер** (после сети/TLS/шрифтов/декодеров; первый шаг — выбор движка с человеком). SMP, dynamic linking, signals/fork развиваются независимо и не блокируют UP software-composited desktop.

## 31. Решения человека (агент не выбирает сам)

Эти развилки меняют контракт/политику системы (новый syscall, wire-формат, модель прав) — по [правилам для агентов](.github/contributors/CLAUDE.md) (§1) их решает человек. Агент оформляет развилку, варианты и рекомендацию, остальное делает сам, не блокируясь.

- **SDK-рантайм (M7.3): свой `no_std+alloc` крейт vs полноценный `std` поверх musl.**
  - A (свой крейт): меньше поверхность, полный контроль над API, строится на готовых `runtime/`+`libdunit`; минус — свой нестандартный API, экосистема Rust `std`-крейтов недоступна.
  - B (`std` над musl): разработчики получают привычный `std` и часть crates.io; минус — `std`-бэкенд тянет широкую POSIX (сеть/signals/процессы) раньше M8 и дорог в поддержке.
  - **Принято 26 октября 2026:** A (`no_std + alloc` поверх `libdunit`). Полноценный `std` не является текущим рантаймом M7. API не должен закрывать возможность отдельного std-порта позднее.

- **wait-set и прочие изменения ABI (M7.1, M8, uname).** Новый syscall(ы) для единого ожидания набора объектов; uname/sysinfo-источник фактов для `dufetch`; сетевые права/handles (ниже) — всё это изменения `abi/*.abi`.
  - **Принято 26 октября 2026:** пользователь разрешил `WaitSetWait` с call-scoped level-triggered ожиданием; реализован под номером 89. Новые ABI-контракты сверх этого решения требуют отдельного согласования.
  - **Рекомендация:** wait-set — первым пунктом M7 (блокирует event loop и сеть); uname-источник — перед миграцией `dufetch`; добавлять строго через `abi/syscalls.abi`+регенерацию с проверкой нумерации.

- **Формат манифеста `dunit.toml` (M7.2).** Поля (`id`, имя, иконка, вид gui/tui/cli, `capabilities`), источник (initrd/`/system`), как его читают launcher/dock/`init`.
  - **Решение принято 26 октября 2026:** `schema_version = 1`, таблица `[application]`, точный Cargo binary target; `capabilities` — уникальные имена из `abi/rights.abi`, без выдачи прав. Полный контракт: `docs/sdk/app-manifest-proposal.md`. GUI discovery реализован; CLI/TUI desktop adapter остаётся открытым.

- **Модель drop/буфера обмена (M7.7): путь vs capability на файл.**
  - **Рекомендация (как в задаче):** путь — для текстовых приёмников (терминал вставляет экранированный путь); для остальных — ограниченный по правам handle на файл на время операции. Это даёт и совместимость с CLI, и контролируемую передачу данных без ambient-доступа к ФС.

- **Вынос оболочки `gui_server` (M7.9): отдельный SDK-клиент vs оставить в компоновщике.**
  - **Рекомендация:** решать ПО ИЗМЕРЕНИЯМ (время кадра, задержка ввода, краш-устойчивость, сложность загрузки). Выносить только если метрики не хуже baseline и падение оболочки не роняет композитор; иначе оставить в `gui_server`, но на общих токенах/теме. Ядро компоновщика в любом случае не переписывать.

- **C-слой GUI (M7.10): тонкая C-обёртка над Rust-рантаймом vs самостоятельный тулкит на C.**
  - **Рекомендация:** обёртка. Одна реализация виджетов/темы/поведения (Rust-рантайм), C-слой тонкий и генерируемый; самостоятельный C-тулкит = второй источник правды и двойная поддержка.

- **`mprotect` и модель исполняемой памяти для `tcc -run` (M7.10).** Компиляция в память + немедленное исполнение требует RW→RX смены прав страницы при W^X — нужен syscall `mprotect`(RX) или эквивалент = изменение ABI.
  - **Рекомендация:** на первом этапе НЕ делать `-run`, оставить компиляцию в файл (`tcc -o a.out` + `posix_spawn`, уже работает). Добавлять исполняемую-память модель только осознанным решением (JIT ослабляет W^X — ограничить правом/capability, не включать глобально).

- **Стабильность C ABI UI-рантайма (M7.10).** Какие C-символы/структуры/версия экспортируются и когда фиксируются.
  - **Рекомендация:** проектировать C ABI рантайма как входное требование M7.3/M7.4 (не пост-фактум); до стабилизации — помечать как нестабильный и версионировать; фиксировать перед миграцией C-программ.

- **Права на сеть (M8).** Новые права-биты в `abi/rights.abi` (сейчас 7: READ/WRITE/MAP/SIGNAL/TRANSFER/DISPLAY_MASTER/INPUT_MASTER) и handle-объекты `NET_DEVICE`/`PACKET_QUEUE` с правами RX/TX/CONTROL/RAW; `netd` — привилегированный init-сервис, обычное приложение получает ограниченный socket-endpoint, raw-пакеты — только диагностике/`netd`.
  - **Рекомендация:** следовать narrowing-only модели существующих handle-прав; raw-доступ по умолчанию запрещён, приложения запрашивают сетевые scopes в `dunit.toml`.

- **Движок браузера (M10, первый шаг).** Портировать существующий облегчённый HTML/CSS-движок vs писать с нуля vs только viewer-slice.
  - **Рекомендация:** решается с человеком, КОГДА проект дойдёт до M10 (не сейчас); до того — довести M8 (сеть/TLS) и уровень шрифтов/декодеров, которые нужны при любом варианте.
