/* POSIX bind/listen/accept/accept4/getsockname checks over isolated E1000. */
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dunit/posix-sockets.h>
#include <dunit/syscall.h>
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#define DEADLINE_MS 25000

static void check(int condition, const char *op)
{
    if (!condition) {
        fprintf(stderr, "[POSIX-LISTEN] FAIL %s errno=%d\n", op, errno);
        exit(1);
    }
}

static uint64_t monotonic_ms(void)
{
    struct timespec ts;
    check(clock_gettime(CLOCK_MONOTONIC, &ts) == 0, "clock_gettime");
    return (uint64_t)ts.tv_sec * 1000 + (uint64_t)ts.tv_nsec / 1000000;
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
            check(monotonic_ms() < deadline, "grant deadline");
            poll(NULL, 0, 20);
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

static int open_socket(int type, uint64_t deadline)
{
    for (;;) {
        int fd = socket(AF_INET, type, 0);
        if (fd >= 0)
            return fd;
        check(errno == EPIPE || errno == EAGAIN || errno == ENOMEM, "wait netd socket bridge");
        check(monotonic_ms() < deadline, "socket bridge deadline");
        poll(NULL, 0, 20);
    }
}

int main(void)
{
    uint32_t grant = receive_grant();
    check(dunit_socket_use_grant(grant) == 0, "install connect grant");
    uint64_t deadline = monotonic_ms() + DEADLINE_MS;

    int srv = open_socket(SOCK_STREAM | SOCK_NONBLOCK, deadline);
    check(srv >= 0, "open tcp listener socket");

    struct sockaddr_in low = {
        .sin_family = AF_INET,
        .sin_port = htons(80),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    check(bind(srv, (struct sockaddr *)&low, sizeof low) == -1 && errno == EACCES, "deny low port");

    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(8080),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    while (bind(srv, (struct sockaddr *)&addr, sizeof addr) != 0) {
        check(errno == EAGAIN || errno == EIO || errno == EPIPE || errno == ETIMEDOUT,
              "wait netd interface");
        check(monotonic_ms() < deadline, "bind deadline");
        poll(NULL, 0, 20);
    }
    struct sockaddr_in bound = {0};
    socklen_t blen = sizeof bound;
    check(getsockname(srv, (struct sockaddr *)&bound, &blen) == 0, "getsockname");
    check(ntohs(bound.sin_port) == 8080 && bound.sin_addr.s_addr == inet_addr("192.0.2.12"), "bound 192.0.2.12:8080");
    check(listen(srv, 4) == 0, "listen");

    int dup_srv = socket(AF_INET, SOCK_STREAM, 0);
    check(dup_srv >= 0, "open second tcp socket");
    check(bind(dup_srv, (struct sockaddr *)&addr, sizeof addr) == -1 && errno == EADDRINUSE, "duplicate port EADDRINUSE");
    close(dup_srv);

    check(accept(srv, NULL, NULL) == -1 && errno == EAGAIN, "empty nonblocking accept EAGAIN");

    int udp = socket(AF_INET, SOCK_DGRAM, 0);
    check(udp >= 0, "open udp socket");
    struct sockaddr_in uaddr = {
        .sin_family = AF_INET,
        .sin_port = htons(9090),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    check(bind(udp, (struct sockaddr *)&uaddr, sizeof uaddr) == 0, "bind udp 9090");
    puts("[POSIX-LISTEN] bind, listen and scope checks passed");

    struct pollfd pfd = { .fd = srv, .events = POLLIN };
    check(poll(&pfd, 1, 20000) == 1 && (pfd.revents & POLLIN), "poll listener POLLIN");

    struct sockaddr_in peer = {0};
    socklen_t plen = sizeof peer;
    int client = accept4(srv, (struct sockaddr *)&peer, &plen, 0);
    check(client >= 0, "accept4 blocking client right after POLLIN");
    check(plen == sizeof peer && peer.sin_addr.s_addr == inet_addr("192.0.2.2") && ntohs(peer.sin_port) == 55111, "accepted peer addr");

    char buf[32] = {0};
    ssize_t n = recv(client, buf, sizeof buf, 0);
    check(n == 12 && memcmp(buf, "hello-listen", 12) == 0, "recv accepted payload");
    check(send(client, "listen-ack", 10, 0) == 10, "send listen-ack");
    check(shutdown(client, SHUT_WR) == 0, "shutdown accepted socket");
    check(recv(client, buf, sizeof buf, 0) == 0, "accepted EOF");
    close(client);
    puts("[POSIX-LISTEN] TCP accept and echo passed");

    struct sockaddr_in usrc = {0};
    socklen_t ulen = sizeof usrc;
    n = recvfrom(udp, buf, sizeof buf, 0, (struct sockaddr *)&usrc, &ulen);
    check(n == 13 && memcmp(buf, "udp-listen-in", 13) == 0, "recvfrom udp 9090");
    check(sendto(udp, "udp-listen-out", 14, 0, (struct sockaddr *)&usrc, ulen) == 14, "sendto udp reply");
    puts("[POSIX-LISTEN] UDP bound echo passed");

    for (;;) {
        int c = accept(srv, NULL, NULL);
        if (c == -1 && errno == EPIPE)
            break;
        check(c == -1 && errno == EAGAIN, "wait revocation");
        check(monotonic_ms() < deadline, "revocation deadline");
        poll(NULL, 0, 20);
    }
    close(udp);
    close(srv);
    puts("[POSIX-LISTEN] revocation rejected listener with EPIPE");
    puts("[POSIX-LISTEN] complete");
    return 0;
}

