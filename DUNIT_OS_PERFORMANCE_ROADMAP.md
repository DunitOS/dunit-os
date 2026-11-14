# Dunit OS — отдельный roadmap производительности и SMP

Создан: 11 ноября 2026. Обновлён: **14 ноября 2026**; реализации и результаты привязаны к отчётам ниже.
Статус: **OP0–OP3 выполнены частично; текущая работа — OP3; SMP dispatch ещё нет**.
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

## Текущее состояние

| Этап | Статус | Подтверждённый результат / оставшаяся работа |
| --- | --- | --- |
| OP0 | Частично | TCG/UP baseline, metadata и checked benchmarks готовы; IPC ping-pong и HPET metadata готовы; KVM reference, часть сцен и полная телеметрия открыты |
| OP1 | Частично | Аудит, CPU-local entry, registry/task locks, pins и exclusive execution leases готовы; cancellation/IRQ model ещё открыт |
| OP2 | Частично | Limine AP boot, APIC/IPI и timer probes: BIOS 1/2/4/8, UEFI 4; HPET/PIT fallback и cross-CPU brackets готовы; timer event model и платформенные gates открыты |
| OP3 | В работе | VFS/backend, PMM, registry/task locks, teardown и dispatch pins готовы; exclusive dispatch/return handoff готовы; checkpoint/register и IPC locks готовы; remote cancel, IRQ и TLB gates открыты |
| OP4 | Подготовка | Exclusive CPU/ticket admission реализован; remote dispatch и параллельного исполнения приложений ещё нет |
| OP5 | Не начат в этом треке | Масштабирование и scheduling latency — после корректного SMP |
| OP6–OP8 | Не закрыты | Новые оптимизации ввода/SDK/рендера/I/O требуют профиля; исторические GUI-правки входят в baseline |
| OP9 | Не закрыт | Итоговая performance/scaling-матрица требует готового SMP и стабильного reference runner |

**Фактический режим:** до 8 аппаратно запущенных vCPU в проверенной TCG-матрице,
`scheduler=up dispatch_cpus=1 APs=parked`. AP обрабатывают изолированные hardware/PMM probes;
приложения и pthread workers всё ещё выполняются последовательно на одном CPU.
KVM на стенде недоступен. На QEMU q35 выбран HPET (шаг счётчика 10 нс),
проверен PIT fallback; timer events пока PIT 100 Hz. Старые PIT-замеры и host/QMP
stream time не доказывают ускорение, per-event tail latency или input-to-photon.

Закрытая галочка означает завершённую узкую реализацию с отчётом, а не приёмку
всего этапа. Смешанные пункты ниже разделены: закрытые подзадачи не снимают
оставшиеся SMP safety/performance gates.

## Ближайший порядок работ

1. **OP3: execution/lifecycle и IRQ progress.** Registry и task уже имеют
   реальные locks и владеющие pins; нет приватного static mutable Vec.
   Exclusive execution/return handoff уже реализован: Ready до завершения HAL
   не разрешает повторный запуск. Теперь закрыть remote cancellation/stop ack,
   predicate/check/register уже защищён notification checkpoint; остаются remote
   stop/cancel и interruptible syscall boundaries.
   Pin процесса не является атомарным правом исполнения или защитой mapping.
2. Согласовать порядок process/task ↔ VFS/ProcFs, wait queues, IPC/handles,
   PMM/VMM и драйверов. Закрыть оставшиеся reentry и lifecycle/cancellation пути.
3. Обеспечить atomic check/block/wake, interruptible syscall boundaries и прогресс
   IPI; внедрить active CPU masks и acknowledged TLB shootdown до reuse страниц.
4. **OP4:** атомарный Ready → Running(cpu), remote wakeup/reschedule, idle и
   миграция GPR/FPU/TLS; затем доказать overlapping execution и корректность
   приложений/потоков на 1/2/4/8 CPU.
5. Параллельно закрывать неполные измерения OP0; KVM scaling и численные
   performance gates выполнять на подходящем reference runner после OP4.

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

[Первый baseline и ограничения](docs/performance/op0-baseline.md), коммит `f1aae67`.

- [x] Стенд GUI wheel/short/long/drag с сохранением исходных correctness assertions,
  warmup и пятью измеренными повторами; отдельный wheel-burst.
- [x] Нативные CPU/pthread workers 1/2/4/8 с одинаковым объёмом работы и checksum,
  allocator и MemFS I/O с проверкой данных; warmup + пять повторов.
- [x] Opt-in metadata: source/image hashes, tools, QEMU command/accel, host limits,
  requested/emulated/kernel-online CPU отдельно от dispatch CPU; результаты JSON.
- [x] Разделить coarse PIT elapsed, host wall time и неизмеренные показатели в
  отчётах. [Queue](docs/performance/scheduler-queue.md) и
  [dispatch](docs/performance/dispatch-ownership.md) имеют checked до/после серии.
- [x] [Нативный IPC ping-pong](docs/performance/progress-foundation.md): два процесса,
  warmup + пять серий по 1 000 round trips с sender/order/data/checksum checks.
- [ ] Завершить набор сцен: idle/startup, loaded/multi-client и
  фиксированная игровая нагрузка. Pointer/GUI correctness уже проверяется;
  это не полноценный performance baseline этих сцен.
- [ ] Зафиксировать KVM reference machine, шум/бюджеты и overhead профилирования;
  измерить CPU accounting, per-event latency tails, очереди/ожидания и RSS.
- **Приёмка:** одинаковые данные/пиксели/счётчики в повторах, явная оценка шума;
  одна команда воспроизводит сценарий через `qemu_test.py`. Если KVM недоступен,
  baseline TCG допустим, но KVM performance gate остаётся открытым.

### OP1 — аудит SMP и CPU-local контекст

- [x] [Реестр shared state, lock order и lifetime](docs/performance/smp-audit.md).
  Устранение перечисленных гонок не считается завершённым самим аудитом.
- [x] CPU-local current PID, syscall scratch/stack/escape pointers, scheduling
  flags и user FPU pointer; private GDT/TSS/IST и AP stacks (`3ccaf6c`).
  FS/TLS, SWAPGS, syscall/IRQ/exception и возврат проверены native/DF/FPU тестами.
- [x] CPU-bound guards и корректный `Sync` bound разделяемого guard (`0142ba9`).
- [x] Публичные current process/thread getters со static-ссылками заменены
  [non-escaping scopes](docs/performance/process-access.md), `a6f0d06`.
- [x] [Приватные current getters удалены](docs/performance/task-scopes.md):
  внутренние FD/TLS/futex/dispatch/lifecycle paths используют scopes; мутации
  current и explicit PID имеют общий fd notification boundary. Signal delivery
  отделена от borrow отправителя; реальный registry/task mutex ещё открыт.
- [x] Dispatch передаёт в HAL скопированный контекст и владеющие resource pins,
  [без Process-ссылки через userspace](docs/performance/dispatch-ownership.md), `236e83d`.
- [x] [Registry/task ownership и настоящие locks](docs/performance/process-registry.md):
  scoped metadata guard, Arc-owned Process mutex, revalidation после lock и task
  pin через dispatch; retire/destructors вне guards, без static mutable Vec.
- [ ] Завершить execution/cancellation и IRQ/preemption nesting/idle model для SMP.
  Task pin не является Running(cpu) claim или правом запустить retired frame.
- **Приёмка:** реестр с владельцами и инвариантами; UP регрессии зелёные,
  нет общих writable entry scratch/стеков. Полная приёмка ждёт оставшихся ownership
  gates; AP пока не исполняют рабочие задачи.

### OP2 — boot дополнительных CPU, APIC и межпроцессорные прерывания

[Bring-up и матрица](docs/performance/smp-bringup.md), коммит `3ccaf6c`.

- [x] Limine MP handshake; AP с private stacks, GS/GDT/TSS/IST, IDT register,
  syscall MSR и x87/SSE. Логические индексы отделены от APIC IDs.
- [x] Изолированные xAPIC/IPI round trips, EOI/spurious handling, BSP PIC routing
  и PIT-калиброванные local timer probes; после проверки AP паркуются без
  постоянных timer wakeups. BIOS 1/2/4/8 и UEFI 4 vCPU TCG проверены.
- [x] [Глобальный HPET clocksource и PIT fallback](docs/performance/progress-foundation.md):
  ACPI/checksum/64-bit/progress validation, IRQ-off progress, реальные AP clock
  brackets, раздельная clock/event metadata; TSC cross-CPU не предполагается.
- [ ] Завершить timer event model и точные wakeups. События пока PIT 100 Hz;
  шаг HPET не является wakeup/dispatch latency или приёмкой всех платформ.
- [ ] Расширить платформенную проверку (включая x2APIC и KVM/железо);
  xAPIC-матрица не считается доказательством всех APIC-режимов.
- **Приёмка:** AP boot/park и реальные work/timer/IPI checks выполнены на стенде;
  clock/platform gates остаются открытыми. Это ещё не SMP scheduler.

### OP3 — память и синхронизация для одновременного исполнения

Выполненные части:

- [x] Per-operation VFS path buffers, включая оба endpoint rename (`2e5d0c6`).
- [x] [Владеющие backend и scoped VFS](docs/performance/vfs-ownership.md), `b236fac`:
  общий backend lock для mounts/open files/capabilities, lifetime/final-close proof,
  удаление raw backend pointers и глобальной mutable VFS-ссылки.
- [x] [Отделённая очистка process resources](docs/performance/process-resources.md),
  `9e9b4c6`: fd/VM/capability batches, наследование fd с rollback, release вне
  lifecycle mutation; kill закрывает fd/pipe до wait; fd snapshots владеющие.
- [x] [Queue → process-table edge удалён](docs/performance/scheduler-queue.md),
  `ea5720b`: state checks вне queue lock, generation-safe validation/requeue,
  ограниченный обход без временных allocations на dispatch path.
- [x] [Scoped current access и deferred fd notifications](docs/performance/process-access.md),
  `a6f0d06`: внешние чтения/мутации и thread wait-set без escaping Process refs;
  fd rescan после завершения borrow, включая callback error paths.
- [x] [Dispatch backing и active-root lifetime](docs/performance/dispatch-ownership.md),
  `236e83d`: Arc-owned stack/FPU/locked AddressSpace, copied HAL input,
  удержание active root и восстановление caller PID/CR3/stack на ошибках.
  Canonical kernel-half refresh сохранён; private-page/root lifetime проверен.
- [x] [Wake callbacks вне wait queue lock](docs/performance/wait-queue.md):
  IPC/pipe/futex/poll/wait-set/timer снимают регистрацию до обращения к task и
  scheduler; generation cutoff исключает новые регистрации из текущего прохода.
  Scheduler admission выполняется после завершения mutable table borrow.
- [x] [Futex admission без обратного захвата task/AddressSpace](docs/performance/futex-admission.md):
  registry lookup и mapping lock до очереди; атомарный compare остаётся вместе
  с регистрацией, clear-child-tid публикуется через Release; borrowed word view
  не переживает guard. Это не межъядерное владение task или cancellation.

- [x] [PMM bitmap/accounting под настоящим leaf lock](docs/performance/pmm-concurrency.md):
  единая транзакция alloc/contiguous/free/stats, однократный handoff boot bitmap;
  host 80 000 смешанных транзакций и реальные BSP/AP cohorts на 1/4/8 CPU.
  AP выполняют только этот boot probe и снова паркуются; task dispatch остаётся UP.

- [x] [Scoped registry/task locking и pins](docs/performance/process-registry.md):
  lookup копирует Arc до task lock, проверяет identity после ожидания; current,
  explicit PID, TLS/join/reap/kill, preemption и futex переведены совместно.
  Dispatch frame удерживает весь task, без mutex guard через HAL/IRET.

- [x] [Notification checkpoint и registration identity](docs/performance/progress-foundation.md):
  IPC/pipe/poll/wait-set check/register защищены от выигравшего producer;
  delayed wake не затрагивает новый park. Host 8 000 forced gaps + native proof.
- [x] [IPC guard и очередь FIFO](docs/performance/progress-foundation.md): настоящий
  lock вместо static mutable manager, notifications после guards, packet identity,
  target handle insertion/queue publication в одной task transaction.

Оставшиеся обязательные gates:

- [ ] Полный lock ranking/reentry audit для process/task, VFS/ProcFs, wait queues,
  IPC/handles, heap/PMM/VMM и драйверов. Удалённые отдельные lock edges не закрывают
  весь порядок. Не спать под spinlock и не ждать ack под lock получателя.
- [ ] Безопасные interruptible syscall boundaries и гарантированный прогресс IPI
  вместо переноса бесконечных IRQ-off окон на несколько CPU.
- [ ] Завершить remote lifecycle/cancellation/stop для потоков и ожиданий,
  remote runqueue notification и порядок освобождения. Check/register и wait
  identity уже защищены; это не остановка выполняющегося CPU.
- [ ] Закрыть общий heap/PMM/shared VM/handles/device lock order и последний reader.
  PMM операции защищены отдельно; это не доказательство всей цепочки allocator/VM.
  Pin AddressSpace не является pin каждой mapping или shared-frame bookkeeping.
- [ ] Active CPU masks, generation/ack TLB shootdown до reuse страниц: unmap,
  mprotect/W^X, shared mappings, exit, миграция и гонка входа в address space.
- [ ] Выполнить cross-CPU memory/lifetime stress и fault injection после реализации
  этих механизмов. Текущие UP/native регрессии не заменяют эту матрицу.
- **Приёмка:** нет stale mappings, double free, deadlock и утечки прав;
  общий AP userspace dispatch запрещён до проверки перечисленных инвариантов.

### OP4 — настоящий SMP scheduler и потоки

- [x] [Exclusive execution admission и HAL return handoff](docs/performance/task-execution.md):
  reservation с CPU/generation и task identity, Running перед HAL, запрет повторного
  запуска до завершения return/cleanup; retirement checks для всего owner/thread group.
  Host actual leases: 8 000 admissions; native и UP regressions. Это ещё не remote
  cancellation и не одновременное исполнение приложений на нескольких CPU.
- [ ] Включить несколько одновременно Running tasks на AP после оставшихся OP3
  gates; сохранить уже установленный exclusive `Ready → Running(cpu)` claim.
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
нескольких совместимых проверок. CPU задавать явно через harness
(`--cpus 4`; raw `-smp` оставлен для специальных конфигураций и не совмещается
с `--cpus`); это параметр эксперимента, не реализация SMP.

Для каждого закрытого пункта нужны commit, команда, raw result, correctness
assertions и краткий вывод с ограничениями. Новые syscall/ABI, публичные stats,
gui-v1, scheduling policy и иные изменения системного контракта согласовываются
перед реализацией по `.github/contributors/CLAUDE.md`; создание этого документа
не является автоматическим одобрением всех будущих ABI/политических решений.
