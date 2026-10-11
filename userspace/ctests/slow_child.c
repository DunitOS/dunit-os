/* Slow child for the reap-order test: sleeps so it is still running when its
 * faster sibling has already exited, then exits with code 3. */
#include <unistd.h>

int main(void)
{
	sleep(1);
	return 3;
}
