/* Child for the posix_spawn test: prints its argv and exits with code 7. */
#include <stdio.h>

int main(int argc, char **argv)
{
	printf("[CHILD] argc=%d argv1=%s\n", argc, argc > 1 ? argv[1] : "(none)");
	return 7;
}
