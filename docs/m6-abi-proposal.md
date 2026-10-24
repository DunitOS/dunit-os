# M6: недостающие контракты, ожидающие решения

Статус: пакет разрешён человеком в этой задаче; реализуемый контракт описан в
[native-fd-abi.md](native-fd-abi.md).

База: `36e85ce`, существующие syscall numbers 0–84. Этот документ — предложение,
не разрешение менять ABI. Новые M7 wait-set, uname/sysinfo, SDK runtime,
application manifest, DnD и JIT policy сюда не входят.

## Предлагаемый пакет M6

1. Восстановить native Fcntl=86, Poll=87, Ioctl=88 из потерянной линии,
   сохранив номера и проверив типизированные записи/ограничения. Номер85 не занимать
   автоматически: в той линии он использовался DisplayOwner. POSIX структуры
   переводит musl adapter; ядро принимает Dunit records/flags. Fcntl предоставляет
   fd flags/CLOEXEC/nonblocking и duplication; Poll — bounded readiness/timeout
   по fd с kernel wait queues, не userspace busy loop; Ioctl — документированный
   terminal subset, остальные запросы явно EOPNOTSUPP.
2. SpawnArgs: шестой до сих пор неиспользуемый аргумент задаёт границу строковой
   секции/действий. Нулевое значение сохраняет старый argv/env формат. В новом
   варианте bounded tagged actions (open/close/dup2/chdir) валидируются целиком,
   применяются к новой fd table с refcount, cwd/argv/env и ELF подготавливаются
   до публикации ребёнка. Родительские fd не меняются. Ошибка закрывает все
   временные references; child не запускается. Привилегии не наследуются.
3. PtySpawn: использовать свободный шестой аргумент для argv count в существующем
   строковом block; 0 сохраняет прежний формат env-only. Это позволяет dsh передать
   реальные argv интерактивному ребёнку без обхода через environment или файл.
4. Mmap: поддержать file-backed MAP_PRIVATE через уже имеющиеся fd/offset
   позиции, с eager read, bounds/alignment и W^X. MAP_SHARED/demand paging/COW
   не объявлять реализованными.

## Почему не локальный обход

Редирект через временный dup2 в родителе создаёт гонки между потоками.
Передача argv через специальную environment переменную меняет модель приложений.
Игнорирование CLOEXEC/NONBLOCK или file-actions — ложный успех. Поэтому без
решения по пакету выше эти запросы должны явно отказываться, а не имитироваться.

## Приёмка до объявления готовности

- spawn actions: наследование/закрытие fd, argv/env/cwd, redirection/pipe,
  отрицательные сценарии и отсутствие fd leaks;
- fcntl/poll: CLOEXEC, O_NONBLOCK/EAGAIN, timeout/readiness/EOF/hangup;
- ioctl: terminal subset и честный отказ неизвестным запросам;
- PTY: `lua /path/script.lua`, произвольные args и quoted names в dsh;
- mmap: offset/EOF/permissions/unmap и корректная очистка при ошибке;
- ABI generation, все existing marker contracts и старые callers остаются зелёными.

Рекомендация: разрешить отдельный пакет M6 после просмотра контрактов; решения
M7 и policy tcc -run оставить отдельными, как требует исходное задание.

Уточнение совместимости: новые варианты SpawnArgs/PtySpawn включаются явным
bit-63 флагом прежней bounded длины. Старые вызовы игнорируют r9: старый
syscall5 не обязан обнулять неиспользуемый регистр. Это проверяется raw native
вызовом с r9=u64::MAX, а не предположением о содержимом регистра.

## Дополнение: detached thread exit

Новый 1000-cycle тест выявил удержание 256 KiB kernel stack на каждый завершённый
pthread: musl ждёт clear-child-tid, но не выполняет native ThreadJoin. Независимо
от решения ниже kernel освобождает тяжёлый объект/TLS после выхода из его стека,
сохраняя лёгкий status до join; musl join сохраняет native TID в private Part 2
TCB и вызывает существующий ThreadJoin=30 после clear-child-tid синхронизации.

Для detached потоков предлагается совместимое расширение существующего
ThreadExit=31: высокие 32 бита arg0 равны 1 — явно запрошенный auto-reap,
низкие 32 бита — i32 exit code. Обычные старые i32-коды имеют высокую половину
0 либо 0xffffffff и сохраняют прежнее поведение. Другие значения не создают
неявный detach. Запрос допустим только для дополнительного потока, не для main.
После HAL escape ядро освобождает его ресурсы и удаляет status record, который
по определению detached не имеет joiner. Новый номер syscall не нужен.

Вариант без этого расширения: joinable pthreads полностью используют ThreadJoin,
но detached хранит лёгкие native status records до завершения owner-процесса.
Это остаётся ограничением, а не готовой detach/reclamation semantics.
Рекомендация — разрешить versioned auto-reap как отдельное M6 уточнение;
wait-set, signals и шесть решений M7 не затрагиваются.

Решение человека: versioned auto-reap для detached разрешён 2026-10-24.
