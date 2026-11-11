# Dunit OS — отдельный roadmap производительности и SMP

Дата: 11 ноября 2026. Статус: **в работе: OP0 baseline и OP1/OP2 foundation; SMP dispatch ещё нет**.
Инициатива пользователя: максимальная практическая производительность,
настоящая одновременная работа на нескольких ядрах и отзывчивый desktop/SDK.
Основной [технический roadmap](DUNIT_OS_TECHNICAL_ROADMAP.md) сохраняется;
этот трек не объявляет M7 завершённым и не заменяет его функциональную приёмку.

## Цель и границы

Уменьшить время полезной работы, задержку ввода и разброс времени кадра;
повысить пропускную способность при увеличении числа CPU. Результат оценивается
при одинаковых сценах, качестве изображения и объёме работы. Рост среднего FPS
ценой потери ввода, неправильных пикселей или зависаний результатом не считается.
«Максимум» — измеренная эффективность на зафиксированной платформе, а не обещание
одного FPS для любого железа, разрешения и приложения.

Первые цели: 1/2/4/8 vCPU; GUI, текст, прокрутка, IPC, файловые операции и
CPU-bound программы на Rust/C. Игровая нагрузка — отдельный фиксированный
software-render benchmark; Minecraft включается после появления воспроизводимой
сборки/сцены, без заявления, что порт уже проверен.

SMP означает, что независимые процессы и потоки реально исполняются на разных
CPU одновременно. Число CPU в QEMU, AP-boot и сообщение `online=4` сами по себе
этого не доказывают. Базовый путь — CPU/SMP; GPU рассматривается отдельно после
измерения предела программного рендера. NUMA, CPU hotplug и real-time guarantees
не входят в первый проход.

## Подтверждённая исходная точка до OP1/OP2

| Область | Факт до изменений OP1/OP2 | Следствие для плана |
| --- | --- | --- |
| Планировщик | `kernel/src/process/scheduler.rs`: UP round-robin, одна ready queue, `smp=off` | Параллельного dispatch пока нет |
| Состояние процесса | `kernel/src/process/mod.rs`: глобальный `CURRENT_PID`, таблица с UP-инвариантами и выдачей долгоживущих mutable-ссылок | Нужны CPU-local current и новая модель владения/времени жизни |
| Вход в ядро | `hal/src/syscall.asm`: глобальные `syscall_saved_*`; `hal/src/gdt.c`: общий TSS и его стек | На разных CPU контексты будут перезаписываться |
| CPU bring-up | `kernel/src/cpu.rs` инициализирует FPU; определения MP в `hal/src/limine.h` не равны подключённому SMP boot | Требуется действительный запуск AP и локальная инициализация |
| Время | `kernel/src/clock.rs`: PIT 100 Hz; вход syscall маскирует IF | Шаг 10 мс недостаточен для коротких профилей; долгие секции могут искажать elapsed time |
| GUI | Рендер программный; есть bounded input drain, pacing по deadline, damage и кэши | Сначала профилировать существующий путь, не переделывать сделанное |
| QEMU | `tools/qemu_test.py` обычно не добавляет `-smp`; доступны явный accel и `--qemu-arg` | Матрицу CPU/ускорителя надо фиксировать явно |
| Codegen GUI | Например, `gui_client/Cargo.toml`: `opt-level = "z"`, fat LTO, один codegen unit | Измерить tradeoff скорости и размера; размерный профиль не равен максимальной скорости |

Предыдущие результаты: [непрерывный ввод](docs/performance/continuous-input.md),
[приёмка compositor](docs/performance/gui-server-acceptance.md),
[миграция GUI](docs/sdk/gui-migration.md), [pointer modes](docs/sdk/pointer-modes.md).
Это исторические замеры конкретных сценариев, а не новый baseline всего дерева.
Отчёт `frame ~N ms` не равен чистой стоимости композиции или input-to-photon.

## Правила измерения и целевые бюджеты

- Для каждого результата сохранять commit, toolchain/profile, образ, QEMU version,
  accel, CPU model/topology, RAM, разрешение, effects, scene/seed, число workers,
  host CPU/quota и команды. KVM и TCG — разные серии; `auto` для сравнений не использовать.
- Отдельно мерить wall time, время CPU, ожидание ready queue/IPC/FrameDone,
  ввод→обработка→публикация, время кадра, p50/p95/p99, missed deadlines,
  queue high-water/loss, bytes/pixels copied, allocations, cache hit rate и RSS.
  Объём работы и correctness assertions сохранять рядом с числами. Вложенные
  elapsed stages не складывать как независимое CPU time; недоступные метрики
  отмечать как «не измерены», а не заполнять нулями.
- Ускоренный fixed-size прогон сравнивать с 1 CPU: `S(N)=T(1)/T(N)`,
  `E(N)=S(N)/N`. Отдельно проверять throughput при N независимых заданиях.
  Нельзя ускорять тест уменьшением сцены или смешивать эти два вида scaling.
- Warmup и минимум 5 повторов для сравнения; при шуме увеличить число повторов
  и дать интервалы/сырые результаты. TCG — прежде всего correctness/fallback;
  заявления об ускорении SMP требуют KVM/реального железа с достаточными host cores.
- Профилирование агрегированное и выключаемое; оценить его собственную стоимость.
  Не печатать serial на каждый input/syscall/frame в измеряемом hot path.

**Цели, ещё не результаты:** на выбранной KVM-платформе и desktop 1920×1080
начальный ориентир — 60 Hz (16,67 мс на весь кадр), p95 ввода→публикация ≤33 мс,
p99 ≤50 мс в фиксированной обычной UI-сцене, включая конкурентную CPU-нагрузку.
Частоту/точный состав сцены и допустимую долю missed deadlines зафиксировать в OP0.
Публикация не считается физическим scanout: без соответствующей телеметрии
input-to-photon не заявлять. Для игр, blur и больших разрешений — свои бюджеты.

Начальный ориентир независимого CPU-bound scaling: ≥1,5× на 2 CPU и ≥2,5× на
4 CPU; 8 CPU измеряется отдельно. Это предлагаемые цели для OP0, не выдуманные
результаты и не универсальные требования к последовательному приложению.

## Этапы

### OP0 — воспроизводимый baseline и измерительные инструменты

Первый TCG/UP baseline и стенд готовы: [результаты и ограничения](docs/performance/op0-baseline.md).
KVM/reference и непокрытые метрики остаются открытыми.

- [ ] Подготовить короткий набор сцен: idle, ввод/scroll/selection, pointer lock,
  text-heavy окно, несколько клиентов, boot/app startup, IPC ping-pong, CPU workers,
  allocator и I/O.
- [ ] Снять текущий UP baseline и стоимость отдельных стадий; сохранить сырые
  данные и форму итогового JSON. Определить KVM reference machine и бюджеты.
- [ ] Развести clocksource, timer events и host wall time. До точных часов не
  публиковать «наносекундную» точность из PIT/serial/QMP polling.
- **Приёмка:** одинаковые данные/пиксели/счётчики в повторах, явная оценка шума;
  одна команда воспроизводит сценарий через `qemu_test.py`. Если KVM недоступен,
  baseline TCG допустим, но KVM performance gate остаётся открытым.

### OP1 — аудит SMP и CPU-local контекст

- [x] Составить реестр shared state, lock order, IRQ/preemption context и lifetime:
  процессы/threads, PMM/VMM/heap, IPC, handles/fd, VFS, драйверы и serial.
  [Реестр OP1](docs/performance/smp-audit.md); устранение перечисленных гонок — отдельная работа.
- [ ] Заменить UP-зависимые mutable-ссылки безопасным владением/guard-периодами.
  Отключение IRQ на одном CPU не защищает данные от другого CPU; `Box` не
  устраняет aliasing, use-after-free и гонки изменения объекта.
- [~] Ввести CPU-local current task, syscall scratch/стек, scheduler flags,
  GDT/TSS/IST, IRQ/preemption nesting, idle context и FPU state. Проверить
  FS/TLS и схему GS/SWAPGS на syscall, IRQ, exception и возврате в userspace.
  CPU-local entry/flags/FPU pointers, private GDT/TSS/IST и AP idle stacks реализованы;
  параллельный scheduler и ownership shared state не включены.
- **Приёмка:** реестр с владельцами и инвариантами; UP регрессии зелёные,
  нет общих writable entry scratch/стеков. AP пока не исполняют рабочие задачи.

### OP2 — boot дополнительных CPU, APIC и межпроцессорные прерывания

- [x] Выбрать и документировать boot-контракт: основной кандидат — Limine MP
  handshake; отдельный SIPI trampoline только при подтверждённой необходимости.
- [x] Поднять AP с отдельными стеками/CPU-local данными, local interrupt state,
  IDT/TSS, syscall MSR и FPU. Реальные CPU IDs/topology не подменять индексами APIC.
- [~] Реализовать необходимый local APIC/IPI и маршрутизацию device IRQ с EOI,
  spurious handling и совместимым BSP/UP fallback. Калибровать локальные timers;
  быстрый clocksource выбирать по проверенным CPUID/платформенным возможностям.
  Для TSC проверить межъядерную согласованность/монотонность и ordering чтений,
  иначе использовать подходящий fallback; timekeeping не равен timer interrupt.
  APIC/IPI, BSP PIC routing и timer probes проверены на 1/2/4/8 vCPU TCG;
  clocksource остаётся PIT, high-resolution/TSC validation не выполнены.
- **Приёмка:** 1/2/4/8 CPU доходят до контролируемого park/idle; IPI ping/ack и
  локальные таймеры работают на каждом CPU. Это **ещё не готовый SMP scheduler**.

[Отчёт bring-up и регрессии](docs/performance/smp-bringup.md).

### OP3 — память и синхронизация для одновременного исполнения

- [ ] Сделать shared kernel paths корректными при нескольких CPU: lock ranking,
  CPU-local IRQ save/restore, preemption guards, atomics с обоснованным ordering.
  Не спать под spinlock; не ожидать IPI-ack, удерживая нужный получателю lock.
- [ ] Определить безопасные interruptible границы syscall после entry на свой
  kernel stack. IPI/shootdown должны иметь гарантированный прогресс; нельзя
  переносить на SMP бесконечные IRQ-off окна и циклические ожидания CPU.
- [ ] Защитить process/thread lifecycle, allocator/PMM/page tables, IPC и handles.
  Описать порядок публикации, последнего reader и освобождения объектов.
- [ ] Реализовать tracking активных CPU адресного пространства и TLB shootdown
  с завершённым ack до повторного использования страниц; покрыть unmap,
  mprotect/W^X, shared mappings, миграцию, exit и гонку входа в address space.
- **Приёмка:** cross-CPU memory/lifetime stress и fault-injection не дают stale
  mappings, двойного освобождения, deadlock или утечки прав. Нельзя разрешать
  общий userspace dispatch до проверки этих инвариантов.

### OP4 — настоящий SMP scheduler и потоки

- [ ] Разрешить несколько одновременно Running tasks; атомарно/под lock
  закреплять `Ready → Running(cpu)`, исключив исполнение одного TID на двух CPU.
- [ ] Реализовать remote wakeup/reschedule IPI, race-safe block/wake/deadlines,
  CPU-local idle и сохранение GPR/FPU/TLS при вытеснении и миграции.
- [ ] Первый корректный scheduler может иметь общую очередь, но kernel-wide lock,
  сериализующий весь syscall/runtime, не считается завершением производительности.
- **Приёмка:** независимые CPU workers и pthreads выполняются на разных CPU;
  проверить overlapping execution и полезный результат, не только номера CPU.
  Shared-memory, mutex/condvar/futex, IPC, join/detach, kill/exit и FPU/TLS stress
  проходят на 1/2/4/8 CPU. Отдельно показать KVM scaling относительно 1 CPU.

### OP5 — масштабирование scheduler и предсказуемая задержка

- [ ] По профилю убрать очередь/lock contention: per-CPU ready queues, remote
  enqueue и ограниченная балансировка; измерить миграции и cache locality.
- [ ] Добавить проверяемое CPU accounting и wakeup→run latency; сократить долгие
  IRQ-off/непрерываемые секции. Больше timer IRQ само по себе не оптимизация.
- [ ] Уточнить scheduling policy для интерактивной работы без starvation фоновых
  задач; idle/deadline wakeups и tickless-путь вводить после корректных timers.
- **Приёмка:** GUI под насыщением CPU остаётся в бюджете OP0; нет голодания и
  потерянных wakeups, idle не превращается в busy polling, масштабирование лучше
  общей очереди либо измеренно подтверждено, что она пока не bottleneck.

### OP6 — ввод, IPC и SDK без лишней сериализации

- [ ] Разложить latency по kernel queue → scheduling → IPC → client update/render
  → commit/composition. Измерять глубину очередей и восстановление после burst.
- [ ] Сократить лишние wakeups/copies/syscalls; ограниченно объединять обновления
  и публиковать состояние на кадр, сохраняя порядок, все key/button transitions,
  detent units, DnD/clipboard и корректную обработку LOST_BEFORE.
- [ ] Если синхронный FrameDone ограничивает поток, проектировать bounded
  frames-in-flight/backpressure с явным владением буферами и отменой при resize.
  Опубликованные буферы остаются immutable до освобождения последним reader.
- **Приёмка:** p95/p99 и backlog лучше baseline на 1 CPU и SMP; точные текст,
  selection, undo, transfers, cursor lock, resize и frame completion не изменены.
  Изменения публичного протокола сначала оформляются отдельным контрактом.

### OP7 — CPU renderer, текст и полезная параллельная работа SDK

- [ ] Профилировать damage/occlusion, layout/diff, glyph/text caches, SVG/icons,
  ARGB/blur и bandwidth. Убирать повторные layout/allocations и полные repaint
  там, где доказана эквивалентность; сохранять ограниченность кэшей.
- [ ] Сравнить release profiles/LTO/codegen по скорости, размеру и воспроизводимости;
  подтвердить выигрыш hot paths и сохранить утверждённый бюджет образа/ESP.
- [ ] Добавлять SIMD/быстрые blit только с CPU feature checks и scalar fallback.
  До AVX проверить CPUID/OSXSAVE и сохранение расширенного контекста scheduler.
- [ ] После SMP и owning thread/sync API SDK — ограниченный worker pool для
  независимых CPU jobs/tiles. UI state, IPC inbox и публикация остаются у владельца;
  immutable snapshots, dirty regions, blur halo и порядок blending обязательны.
- **Приёмка:** scalar/optimized и single/parallel варианты дают согласованные
  пиксели, ограниченную память и выигрыш на целевой сцене; короткие jobs остаются
  serial, если синхронизация дороже работы. Для игры мерить также время её
  simulation/rasterization, а не приписывать весь кадр compositor.

### OP8 — память, файлы и I/O по выявленному профилю

- [ ] Измерить allocator contention/fragmentation, cache misses/false sharing,
  IPC/shared-buffer копии и RSS; применять batching, locality или per-CPU caches
  с лимитами и reclaim только при подтверждённом bottleneck.
- [ ] Оптимизировать VFS lookup, read buffering и block I/O, сохраняя coherence,
  права, partial I/O, ошибки и durability. Не менять формат ФС ради benchmark.
- [ ] Проверить, что syscall/драйвер/копирование большого framebuffer не удерживают
  глобальный lock или выключенные IRQ на весь объём работы.
- **Приёмка:** throughput/latency и память в фиксированных сценариях лучше baseline;
  fsck/recovery, reboot/persistence и memory-pressure regressions остаются зелёными.

### OP9 — итоговая матрица, пределы и защита от регрессий

- [ ] Сверить 1/2/4/8 CPU, KVM и TCG раздельно, cold/warm, idle/loaded,
  штатное разрешение и более тяжёлое поддерживаемое. Состав сцен и effects одинаков.
- [ ] Добавить короткие CI correctness/smoke checks; длинные stress/perf серии
  запускать отдельно. Численный regression gate включать только на стабильном
  reference runner с порогом, обоснованным шумом OP0.
- [ ] Опубликовать до/после: latency, frame distribution, scaling, throughput,
  lock/IRQ-off hotspots, RSS и ограничения. При пределе CPU renderer отдельно
  предложить GPU/virtio-gpu acceleration с сохранением software fallback.
- **Приёмка:** SMP работает и даёт измеримый выигрыш; обычная работа на 1 CPU не
  деградировала без объяснённого tradeoff; достигнуты зафиксированные бюджеты либо
  этап явно остаётся открытым с конкретным bottleneck. AP boot, средний FPS и
  один удачный прогон не закрывают этот roadmap.

## Порядок и экономная проверка

`OP0 → OP1 → OP2 → OP3 → OP4 → OP5 → OP9` — критический SMP-путь.
Изолированный AP boot/park OP2 допускается после CPU-local entry и аудита OP1:
он не обращается к process/VFS/heap shared paths. Полное ownership OP1 и
remote-memory safety OP3 обязательны до любого AP userspace dispatch OP4.
OP6 и последовательные оптимизации OP7/OP8 можно делать после OP0 по профилю,
не дожидаясь SMP. Parallel worker pool OP7 требует OP4 и готовых SDK thread/sync;
итог OP9 включает результаты OP6–OP8 для выбранных обязательных сцен.

Один содержательный этап — отдельный проверенный коммит; большие SMP-этапы
допускают несколько коммитов с самостоятельными инвариантами. Сборки/VM в общем
`build/` сериализуются. Сначала host/unit и kernel compile check с флагами Makefile,
затем минимальный native сценарий; расширять матрицу после успешного узкого прогона.
Все OS builds/boots — через `tools/qemu_test.py`; один образ переиспользовать для
нескольких совместимых проверок. CPU задавать явно через доступный harness
(`--qemu-arg=-smp --qemu-arg=4`); это параметр эксперимента, не реализация SMP.

Для каждого закрытого пункта нужны commit, команда, raw result, correctness
assertions и краткий вывод с ограничениями. Новые syscall/ABI, публичные stats,
gui-v1, scheduling policy и иные изменения системного контракта согласовываются
перед реализацией по `.github/contributors/CLAUDE.md`; создание этого документа
не является автоматическим одобрением всех будущих ABI/политических решений.
