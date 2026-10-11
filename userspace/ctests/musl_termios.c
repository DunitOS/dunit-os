/* termios line discipline + ioctl(TCGETS/TCSETS) over a pty slave.
 *
 * Run as the slave end of a pty by /app/termios_test. Our fd 0/1/2 are routed
 * to the pty rings, so stdin IS a tty. We:
 *   - isatty(0): a pty slave -> 1 (exercises Ioctl TCGETS / ENOTTY routing)
 *   - tcgetattr: read the (default raw) termios the kernel stores for the pty
 *   - enable ICANON|ECHO via tcsetattr, then re-read to confirm it stuck
 *     (round-trip through Ioctl TCSETS -> pty::set_termios -> get_termios)
 *   - announce readiness on stdout so the master only feeds input afterwards
 *   - read one line: in canonical mode the line discipline delivers a whole,
 *     backspace-edited line, released only on newline
 *   - report exactly what we received as GOT:<line>
 *
 * This program's stdout goes to the pty master, not the console, so the master
 * (termios_test) is what prints the verified [TERMIOS] markers to serial. */
#include <termios.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <sched.h>

int main(void)
{
	int tty = isatty(0);

	struct termios t;
	int rt = 0;
	if (tcgetattr(0, &t) == 0) {
		t.c_lflag |= (ICANON | ECHO);
		if (tcsetattr(0, TCSANOW, &t) == 0) {
			struct termios v;
			if (tcgetattr(0, &v) == 0 &&
			    (v.c_lflag & ICANON) && (v.c_lflag & ECHO))
				rt = 1;
		}
	}

	/* Handshake: hand the master our self-check result AND signal that
	 * canonical mode is armed, so it feeds input only from here on. */
	char rdy[24];
	int n = 0;
	memcpy(rdy, "RDY tty=", 8); n = 8;
	rdy[n++] = tty ? '1' : '0';
	memcpy(rdy + n, " rt=", 4); n += 4;
	rdy[n++] = rt ? '1' : '0';
	rdy[n++] = '\n';
	write(1, rdy, n);

	/* Canonical read: one whole line, erase already applied by the ldisc.
	 * We never echo ourselves, so any echoed bytes the master sees are the
	 * kernel's (proves ECHO). The pty read is non-blocking (EAGAIN while the
	 * ring is empty); yield cooperatively, matching the kernel scheduler. */
	char line[64];
	int r;
	for (;;) {
		r = read(0, line, sizeof line);
		if (r >= 0)
			break;
		if (errno == EAGAIN) {
			sched_yield();
			continue;
		}
		r = 0;
		break;
	}

	char out[80];
	memcpy(out, "GOT:", 4);
	int m = 4;
	for (int i = 0; i < r && m < (int)sizeof out; i++)
		out[m++] = line[i];
	write(1, out, m);
	return 0;
}
