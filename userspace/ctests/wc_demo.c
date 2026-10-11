/* Portable POSIX C — compiles and runs identically on Linux and on Dunit OS.
 * Writes a text file, reads it back, and counts lines/words/bytes (a tiny `wc`).
 * Pass a path as argv[1] (defaults to demo.txt in the current directory). */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
	const char *path = (argc > 1) ? argv[1] : "demo.txt";
	const char *text =
		"the quick brown fox\n"
		"jumps over the lazy dog\n"
		"the fox is quick\n";

	FILE *f = fopen(path, "w");
	if (!f) { perror("fopen(w)"); return 1; }
	fputs(text, f);
	fclose(f);

	f = fopen(path, "r");
	if (!f) { perror("fopen(r)"); return 1; }

	long lines = 0, words = 0, bytes = 0;
	int inword = 0, c;
	while ((c = fgetc(f)) != EOF) {
		bytes++;
		if (c == '\n') lines++;
		if (c == ' ' || c == '\n' || c == '\t') inword = 0;
		else if (!inword) { inword = 1; words++; }
	}
	fclose(f);

	printf("wc %s -> %ld lines, %ld words, %ld bytes\n", path, lines, words, bytes);
	return 0;
}
