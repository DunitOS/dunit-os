/* pthread_cond broadcast with multiple waiters (M6-D thread correctness). Three
 * worker threads block in pthread_cond_wait; main broadcasts once. All three
 * must wake. This exercised a deadlock: cond_broadcast/signal with 2+ waiters
 * uses an ordered wake chain built on FUTEX_REQUEUE, which was an unmapped
 * sentinel (-ENOSYS no-op) on Dunit, so the predecessors were never woken and
 * pthread_join hung forever. */
#define _GNU_SOURCE
#include <stdio.h>
#include <pthread.h>
#include <unistd.h>

static pthread_mutex_t mtx = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cv = PTHREAD_COND_INITIALIZER;
static int ready = 0;
static int woken = 0;

static void *worker(void *arg)
{
	(void)arg;
	pthread_mutex_lock(&mtx);
	while (!ready)
		pthread_cond_wait(&cv, &mtx);
	woken++;
	pthread_mutex_unlock(&mtx);
	return 0;
}

int main(void)
{
	pthread_t t[3];
	for (int i = 0; i < 3; i++)
		pthread_create(&t[i], 0, worker, 0);

	/* Let all three workers reach pthread_cond_wait before broadcasting. */
	usleep(200000);

	pthread_mutex_lock(&mtx);
	ready = 1;
	pthread_cond_broadcast(&cv);
	pthread_mutex_unlock(&mtx);

	for (int i = 0; i < 3; i++)
		pthread_join(t[i], 0);

	if (woken == 3)
		printf("[COND] broadcast woke 3 OK\n");
	else
		printf("[COND] broadcast FAIL woken=%d\n", woken);
	printf("[COND] DONE\n");
	return 0;
}
