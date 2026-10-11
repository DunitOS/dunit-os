/* pthread_barrier over the native Dunit futex (M6-E pthread subset). The barrier
 * owner's wait used to issue a raw Linux SYS_futex (a sentinel → ENOSYS) and
 * busy-spin on the fallthrough; it now blocks on the native FutexWait via
 * __futexwait, like every other musl wait path on Dunit. N threads rendezvous
 * at a barrier: the test confirms all N cross it (so nothing deadlocked on the
 * native futex) and exactly one gets PTHREAD_BARRIER_SERIAL_THREAD. */
#include <pthread.h>
#include <stdio.h>

#define N 4

static pthread_barrier_t bar;
static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static volatile int before, after, serial;

static void *worker(void *arg)
{
	(void)arg;
	pthread_mutex_lock(&m); before++; pthread_mutex_unlock(&m);
	int r = pthread_barrier_wait(&bar);
	pthread_mutex_lock(&m);
	if (r == PTHREAD_BARRIER_SERIAL_THREAD) serial++;
	after++;
	pthread_mutex_unlock(&m);
	return 0;
}

int main(void)
{
	if (pthread_barrier_init(&bar, 0, N) != 0) {
		printf("[BARRIER] FAIL init\n");
		return 0;
	}
	pthread_t t[N];
	for (int i = 0; i < N; i++) {
		if (pthread_create(&t[i], 0, worker, 0) != 0) {
			printf("[BARRIER] FAIL create\n");
			return 0;
		}
	}
	for (int i = 0; i < N; i++)
		pthread_join(t[i], 0);

	pthread_barrier_destroy(&bar);
	printf("[BARRIER] before=%d after=%d serial=%d\n", before, after, serial);
	if (after == N && serial == 1)
		printf("[BARRIER] OK\n");
	else
		printf("[BARRIER] FAIL\n");
	return 0;
}
