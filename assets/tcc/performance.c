/* Fixed work and checked results. Trial 0 is warmup; PIT elapsed is coarse. */
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#define TOTAL 4194304u
#define EXPECTED 9007160971231232ull
struct Job {
  unsigned first, last;
  uint64_t sum;
};
static uint64_t now(void) {
  struct timespec t;
  if (clock_gettime(CLOCK_MONOTONIC, &t))
    exit(2);
  return (uint64_t)t.tv_sec * 1000000000ull + t.tv_nsec;
}
static void *worker(void *p) {
  struct Job *j = p;
  uint64_t sum = 0;
  for (unsigned i = j->first; i < j->last; i++)
    sum += ((uint32_t)(i * 1664525u + 1013904223u)) ^ (i >> 7);
  j->sum = sum;
  return 0;
}
static void report(const char *scene, unsigned trial, uint64_t start,
                   uint64_t units, uint64_t checksum) {
  uint64_t elapsed = now() - start;
  char record[256];
  int count = snprintf(record, sizeof record,
                       "[PERF-CORE] "
                       "{\"scene\":\"%s\",\"trial\":%u,\"elapsed_ns\":%llu,"
                       "\"work_units\":%llu,\"checksum\":%llu}\n",
                       scene, trial, (unsigned long long)elapsed,
                       (unsigned long long)units, (unsigned long long)checksum);
  if (count < 0 || count >= (int)sizeof record ||
      write(1, record, count) != count)
    exit(15);
}
static void cpu(unsigned workers) {
  char label[24];
  snprintf(label, sizeof label, "cpu-%u", workers);
  for (unsigned trial = 0; trial < 6; trial++) {
    struct Job jobs[8];
    pthread_t threads[8];
    uint64_t start = now();
    for (unsigned i = 0; i < workers; i++) {
      jobs[i] = (struct Job){TOTAL * i / workers, TOTAL * (i + 1) / workers, 0};
      if (pthread_create(&threads[i], 0, worker, &jobs[i]))
        exit(3);
    }
    uint64_t sum = 0;
    for (unsigned i = 0; i < workers; i++) {
      void *result = (void *)1;
      if (pthread_join(threads[i], &result) || result)
        exit(4);
      sum += jobs[i].sum;
    }
    if (sum != EXPECTED)
      exit(5);
    report(label, trial, start, TOTAL, sum);
  }
}
static void allocator(void) {
  for (unsigned trial = 0; trial < 6; trial++) {
    uint64_t start = now(), sum = 0;
    unsigned char *p[64];
    for (unsigned block = 0; block < 64; block++) {
      for (unsigned i = 0; i < 64; i++) {
        p[i] = malloc(4096);
        if (!p[i])
          exit(6);
        memset(p[i], (unsigned char)i, 4096);
      }
      for (unsigned i = 0; i < 64; i++) {
        for (unsigned j = 0; j < 4096; j++)
          sum += p[i][j];
        free(p[i]);
      }
    }
    if (sum != 528482304ull)
      exit(7);
    report("allocator", trial, start, 16777216, sum);
  }
}
static void io(void) {
  unsigned char *buffer = malloc(65536);
  if (!buffer)
    exit(8);
  for (unsigned trial = 0; trial < 6; trial++) {
    uint64_t start = now(), sum = 0;
    memset(buffer, 7, 65536);
    int fd = open("/tmp/performance.data", O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0)
      exit(9);
    for (int i = 0; i < 16; i++) {
      unsigned done = 0;
      while (done < 65536) {
        ssize_t n = write(fd, buffer + done, 65536 - done);
        if (n <= 0)
          exit(10);
        done += n;
      }
    }
    if (close(fd))
      exit(11);
    fd = open("/tmp/performance.data", O_RDONLY);
    if (fd < 0)
      exit(12);
    for (int i = 0; i < 16; i++) {
      unsigned done = 0;
      while (done < 65536) {
        ssize_t n = read(fd, buffer + done, 65536 - done);
        if (n <= 0)
          exit(13);
        done += n;
      }
      for (unsigned j = 0; j < 65536; j++)
        sum += buffer[j];
    }
    if (read(fd, buffer, 1) != 0 || close(fd) ||
        unlink("/tmp/performance.data") || sum != 7340032ull)
      exit(14);
    report("memfs-io", trial, start, 2097152, sum);
  }
  free(buffer);
}
int main(void) {
  setvbuf(stdout, 0, _IONBF, 0);
  puts("[PERF-CORE] start");
  for (unsigned workers = 1; workers <= 8; workers *= 2)
    cpu(workers);
  allocator();
  io();
  puts("[PERF-CORE] all checks passed");
  return 0;
}
