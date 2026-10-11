/* Anonymous pipe with a BLOCKING reader (M6-D). A reader thread blocks in
 * read() on an empty pipe; main writes "hello", the reader wakes and gets it;
 * main then closes the write end and the reader's next read returns EOF. This
 * exercises the new kernel pipe object AND the syscall-restart path (a blocked
 * read re-executes and recomputes its byte count on wake). */
#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <pthread.h>
#include <string.h>

static int pfd[2];

static void *reader(void *arg)
{
	(void)arg;
	char b[16];
	int n = read(pfd[0], b, sizeof b); /* blocks: pipe starts empty */
	if (n == 5 && memcmp(b, "hello", 5) == 0)
		printf("[PIPE] reader got hello OK\n");
	else
		printf("[PIPE] reader FAIL n=%d\n", n);
	int e = read(pfd[0], b, sizeof b); /* write end closed -> EOF */
	if (e == 0)
		printf("[PIPE] reader eof OK\n");
	else
		printf("[PIPE] reader eof FAIL e=%d\n", e);
	return 0;
}

int main(void)
{
	if (pipe(pfd) != 0) {
		printf("[PIPE] FAIL pipe\n");
		printf("[PIPE] DONE\n");
		return 0;
	}
	pthread_t t;
	pthread_create(&t, 0, reader, 0);
	usleep(200000); /* let the reader block in read() */
	write(pfd[1], "hello", 5);
	usleep(100000);
	close(pfd[1]); /* signal EOF */
	pthread_join(t, 0);
	close(pfd[0]);
	printf("[PIPE] DONE\n");
	return 0;
}
