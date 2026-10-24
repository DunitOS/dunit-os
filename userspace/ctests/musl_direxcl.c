/* Static musl test for the fs: rewinddir + O_EXCL additions (M6-A, tcc-prep).
 *
 * rewinddir: our opendir snapshots the directory and tracks the cursor in the
 * snapshot (dir->fd = -1), so rewinddir must reset the snapshot cursor, not just
 * lseek a fake fd. Count /app, rewind, count again — the two counts must match
 * (before the fix the second pass saw 0 entries).
 *
 * O_EXCL: open(O_CREAT|O_EXCL) must succeed when the path is absent and fail with
 * EEXIST when it already exists (atomic create; mkstemp/tcc rely on it). Uses
 * /persist (writable). */
#include <dirent.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>

#define TMP "/persist/excl_probe.tmp"

static int count_dir(DIR *d)
{
	int n = 0;
	struct dirent *e;
	while ((e = readdir(d)) != NULL)
		n++;
	return n;
}

int main(void)
{
	DIR *d = opendir("/app");
	if (!d) { printf("[DIREXCL] FAIL opendir\n[DIREXCL] DONE\n"); return 0; }
	int c1 = count_dir(d);
	rewinddir(d);
	int c2 = count_dir(d);
    rewinddir(d);
    struct dirent *first=readdir(d);
    char first_name[256]={0};
    if(first) snprintf(first_name,sizeof first_name,"%s",first->d_name);
    long cookie=telldir(d);
    struct dirent *second=readdir(d);
    char second_name[256]={0};
    if(second)snprintf(second_name,sizeof second_name,"%s",second->d_name);
    seekdir(d,cookie);
    struct dirent *again=readdir(d);
    if(first_name[0] && again && !strcmp(again->d_name,second_name)) puts("[DIREXCL] seek cookie OK");
    else puts("[DIREXCL] seek cookie FAIL");
	closedir(d);
	printf("[DIREXCL] rewind c1=%d c2=%d\n", c1, c2);
	if (c1 > 0 && c1 == c2)
		printf("[DIREXCL] rewind OK\n");
	else
		printf("[DIREXCL] rewind FAIL\n");

	unlink(TMP); /* start clean; ignore ENOENT */
	int fd = open(TMP, O_WRONLY | O_CREAT | O_EXCL, 0600);
	if (fd >= 0) {
		close(fd);
		printf("[DIREXCL] excl create OK\n");
	} else {
		printf("[DIREXCL] excl create FAIL errno=%d\n", errno);
	}

	int fd2 = open(TMP, O_WRONLY | O_CREAT | O_EXCL, 0600);
	if (fd2 < 0 && errno == EEXIST) {
		printf("[DIREXCL] excl reject OK\n");
	} else {
		printf("[DIREXCL] excl reject FAIL fd=%d errno=%d\n", fd2, errno);
		if (fd2 >= 0) close(fd2);
	}
	unlink(TMP);

	printf("[DIREXCL] DONE\n");
	return 0;
}
