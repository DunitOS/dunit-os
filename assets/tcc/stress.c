/* A bigger on-device compile: exercises more of tcc's codegen and more of libc,
 * and stresses tcc's own heap with a larger translation unit. Verifiable output
 * (fixed inputs -> fixed numbers) so a marker can confirm correctness. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct point { int x, y; };

static long fib(int n)
{
	if (n < 2)
		return n;
	return fib(n - 1) + fib(n - 2);
}

static int cmp_int(const void *a, const void *b)
{
	int ia = *(const int *)a, ib = *(const int *)b;
	return (ia > ib) - (ia < ib);
}

static int count_primes(int limit)
{
	char *sieve = calloc(limit + 1, 1);
	if (!sieve)
		return -1;
	int count = 0;
	for (int i = 2; i <= limit; i++) {
		if (!sieve[i]) {
			count++;
			for (int j = 2 * i; j <= limit; j += i)
				sieve[j] = 1;
		}
	}
	free(sieve);
	return count;
}

static int manhattan(struct point a, struct point b)
{
	int dx = a.x - b.x, dy = a.y - b.y;
	if (dx < 0) dx = -dx;
	if (dy < 0) dy = -dy;
	return dx + dy;
}

int main(void)
{
	/* dynamic array + qsort */
	int n = 16;
	int *arr = malloc(n * sizeof *arr);
	if (!arr) { printf("[TCC-STRESS] FAIL malloc\n"); return 1; }
	unsigned seed = 12345u;
	for (int i = 0; i < n; i++) {
		seed = seed * 1103515245u + 12345u;
		arr[i] = (int)((seed >> 16) % 1000);
	}
	qsort(arr, n, sizeof *arr, cmp_int);
	int sorted = 1;
	long sum = 0;
	for (int i = 0; i < n; i++) {
		sum += arr[i];
		if (i && arr[i] < arr[i - 1])
			sorted = 0;
	}
	free(arr);

	/* struct math */
	struct point a = { 3, 4 }, b = { -2, 1 };
	int md = manhattan(a, b);

	/* string formatting + compare */
	char buf[64];
	snprintf(buf, sizeof buf, "fib13=%ld md=%d", fib(13), md);
	int str_ok = (strcmp(buf, "fib13=233 md=8") == 0);

	int primes = count_primes(100);

	printf("[TCC-STRESS] primes=%d fib20=%ld sumlen=%ld sorted=%d str=%d\n",
	       primes, fib(20), sum, sorted, str_ok);
	if (primes == 25 && fib(20) == 6765 && sorted && str_ok)
		printf("[TCC-STRESS] OK\n");
	else
		printf("[TCC-STRESS] FAIL\n");
	return 0;
}

