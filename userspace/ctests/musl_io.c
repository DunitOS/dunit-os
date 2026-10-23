/* Regression for libc's bounded native I/O: no large request may be lost. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/uio.h>
#include <errno.h>
#define SIZE (200 * 1024 + 31)
static int verify(const char *path, const unsigned char *expected, unsigned char *actual)
{
    FILE *f = fopen(path, "r");
    if (!f) return 0;
    size_t n = fread(actual, 1, SIZE, f);
    int extra = fgetc(f);
    int closed = fclose(f);
    return n == SIZE && extra == EOF && !closed && !memcmp(expected, actual, SIZE);
}
int main(void)
{
    unsigned char *data = malloc(SIZE), *readback = malloc(SIZE);
    const char *path = "/tmp/musl-io-regression";
    if (!data || !readback) goto fail;
    for (size_t i = 0; i < SIZE; ++i) data[i] = (unsigned char)(i * 37 + i / 251);
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0) goto fail;
    size_t done = 0;
    while (done < SIZE) {
        ssize_t n = write(fd, data + done, SIZE - done);
        if (n <= 0 || (size_t)n > SIZE - done) { close(fd); goto fail; }
        done += (size_t)n;
    }
    if (close(fd) || !verify(path, data, readback)) goto fail;
    FILE *file = fopen(path, "w");
    if (!file) goto fail;
    size_t wrote = fwrite(data, 1, SIZE, file);
    int closed = fclose(file);
    if (wrote != SIZE || closed || !verify(path, data, readback)) goto fail;
    fd = open(path, O_WRONLY | O_TRUNC);
    if (fd < 0) goto fail;
    struct iovec vec[2] = {{data, SIZE / 2}, {data + SIZE / 2, SIZE - SIZE / 2}};
    ssize_t total = writev(fd, vec, 2);
    closed = close(fd);
    if (total != SIZE || closed || !verify(path, data, readback)) goto fail;
    errno = 0;
    if (write(-1, data, SIZE) != -1 || errno != EBADF) goto fail;
    if (remove(path)) goto fail;
    errno = 0;
    fd = open(path, O_RDONLY);
    if (fd >= 0) { close(fd); goto fail; }
    if (errno != ENOENT) goto fail;
    free(data); free(readback);
    puts("[MUSL-IO] large read/write/writev/stdio/remove OK");
    return 0;
fail:
    free(data); free(readback);
    puts("[MUSL-IO] FAIL");
    return 1;
}
