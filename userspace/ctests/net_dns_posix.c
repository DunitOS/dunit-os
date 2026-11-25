/* Application DNS (getaddrinfo/freeaddrinfo/gai_strerror) checks over E1000. */
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dunit/posix-sockets.h>
#include <dunit/syscall.h>
#include <errno.h>
#include <netdb.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

#define DEADLINE_MS 25000

static void check(int condition, const char *op)
{
    if (!condition) {
        fprintf(stderr, "[POSIX-DNS] FAIL %s errno=%d\n", op, errno);
        exit(1);
    }
}

static uint64_t monotonic_ms(void)
{
    struct timespec ts;
    check(clock_gettime(CLOCK_MONOTONIC, &ts) == 0, "clock_gettime");
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
}

static void pause_briefly(uint64_t deadline)
{
    uint64_t now = monotonic_ms();
    check(now < deadline, "deadline exhausted");
    poll(NULL, 0, 20);
}

static uint32_t receive_grant(void)
{
    const char *env = getenv("DUNIT_NET_SUPERVISOR");
    check(env && *env, "DUNIT_NET_SUPERVISOR");
    uint32_t supervisor = (uint32_t)strtoul(env, NULL, 10);
    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    for (;;) {
        uint8_t payload[256];
        uint32_t sender = 0, attachment = 0;
        long n = syscall(SYS_IPC_RECEIVE_HANDLE, (long)payload, (long)sizeof payload,
                         (long)&sender, (long)&attachment, 0L, 0L);
        if (n == -1 && errno == EAGAIN) {
            pause_briefly(deadline);
            continue;
        }
        check(n >= 0, "receive grant");
        if (attachment)
            syscall(SYS_HANDLE_CLOSE, (long)attachment, 0L, 0L, 0L, 0L, 0L);
        if (sender == supervisor && !attachment && n == 4) {
            uint32_t grant = (uint32_t)payload[0] | ((uint32_t)payload[1] << 8) |
                             ((uint32_t)payload[2] << 16) | ((uint32_t)payload[3] << 24);
            check(grant != 0, "nonzero grant");
            return grant;
        }
    }
}

int main(void)
{
    uint32_t grant = receive_grant();
    check(dunit_socket_use_grant(grant) == 0, "install connect grant");

    struct addrinfo hints = { .ai_family = AF_INET, .ai_socktype = SOCK_STREAM, .ai_flags = AI_NUMERICHOST | AI_NUMERICSERV };
    struct addrinfo *res = NULL;
    check(getaddrinfo("192.0.2.10", "5432", &hints, &res) == 0 && res != NULL, "numeric getaddrinfo");
    struct sockaddr_in *sa = (struct sockaddr_in *)res->ai_addr;
    check(ntohs(sa->sin_port) == 5432 && sa->sin_addr.s_addr == inet_addr("192.0.2.10"), "numeric sockaddr");
    freeaddrinfo(res);

    struct addrinfo bad_hints = { .ai_family = AF_UNIX };
    check(getaddrinfo("du.net.test", "80", &bad_hints, &res) == EAI_FAMILY, "reject AF_UNIX");

    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    struct addrinfo tcp_hints = { .ai_family = AF_INET, .ai_socktype = SOCK_STREAM, .ai_flags = AI_CANONNAME };
    for (;;) {
        int rc = getaddrinfo("du.net.test", "5432", &tcp_hints, &res);
        if (rc == 0)
            break;
        check(rc == EAI_NONAME || rc == EAI_AGAIN || rc == EAI_FAIL, "wait netd DNS readiness");
        pause_briefly(deadline);
    }
    sa = res ? (struct sockaddr_in *)res->ai_addr : NULL;
    check(sa != NULL && ntohs(sa->sin_port) == 5432 &&
          sa->sin_addr.s_addr == inet_addr("192.0.2.10") &&
          res->ai_canonname != NULL && strcmp(res->ai_canonname, "du.net.test") == 0,
          "resolved du.net.test");

    int fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    check(fd >= 0, "blocking TCP socket from addrinfo");
    check(connect(fd, res->ai_addr, res->ai_addrlen) == 0, "connect via addrinfo");
    freeaddrinfo(res);

    struct addrinfo *cached = NULL;
    check(getaddrinfo("du.net.test", "http", NULL, &cached) == 0 && cached != NULL, "cached getaddrinfo + service");
    check(cached->ai_next != NULL, "default hints emit stream and dgram");
    check(ntohs(((struct sockaddr_in *)cached->ai_addr)->sin_port) == 80, "http service port");
    freeaddrinfo(cached);
    puts("[POSIX-DNS] getaddrinfo resolution and cache passed");

    int nx = getaddrinfo("missing.du.net.test", "80", &tcp_hints, &res);
    check(nx == EAI_NONAME && gai_strerror(nx) != NULL, "NXDOMAIN EAI_NONAME");
    puts("[POSIX-DNS] NXDOMAIN and gai_strerror passed");

    check(send(fd, "dns-tcp-ok", 10, 0) == 10, "send via resolved TCP socket");
    char buf[32] = {0};
    ssize_t got = recv(fd, buf, sizeof buf, 0);
    check(got == 11 && memcmp(buf, "dns-tcp-ack", 11) == 0, "recv via resolved TCP socket");
    check(recv(fd, buf, sizeof buf, 0) == 0, "EOF via resolved TCP socket");
    check(shutdown(fd, SHUT_WR) == 0, "shutdown resolved TCP socket");
    check(close(fd) == 0, "close resolved TCP socket");
    puts("[POSIX-DNS] connect and TCP exchange passed");

    for (;;) {
        int rc = getaddrinfo("uncached.du.net.test", "80", &tcp_hints, &res);
        if (rc == EAI_FAIL)
            break;
        if (rc == 0)
            freeaddrinfo(res);
        pause_briefly(deadline);
    }
    check(syscall(SYS_HANDLE_CLOSE, (long)grant, 0L, 0L, 0L, 0L, 0L) == 0, "close grant");
    puts("[POSIX-DNS] revocation rejected getaddrinfo");
    puts("[POSIX-DNS] complete");
    return 0;
}

