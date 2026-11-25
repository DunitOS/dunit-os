/* POSIX AF_INET6 socket and getaddrinfo verification over dual-NIC E1000. */
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
#include <time.h>
#include <unistd.h>

#define DEADLINE_MS 30000

static void check(int condition, const char *op)
{
    if (!condition) {
        fprintf(stderr, "[NET-IPV6-POSIX] FAIL %s errno=%d\n", op, errno);
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
    check(monotonic_ms() < deadline, "deadline exhausted");
    poll(NULL, 0, 50);
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
    check(dunit_socket_use_grant(grant) == 0, "use_grant");

    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    struct addrinfo hints = { .ai_family = AF_INET6, .ai_socktype = SOCK_STREAM, .ai_flags = AI_CANONNAME };
    struct addrinfo *res = NULL;
    for (;;) {
        int rc = getaddrinfo("posix6.du.net.test", "7000", &hints, &res);
        if (rc == 0)
            break;
        check(rc == EAI_AGAIN || rc == EAI_NONAME || rc == EAI_FAIL || rc == EAI_NODATA, "getaddrinfo retry");
        pause_briefly(deadline);
    }
    check(res != NULL && res->ai_family == AF_INET6 && res->ai_addrlen == sizeof(struct sockaddr_in6),
          "AF_INET6 addrinfo");

    int tcp_fd = socket(res->ai_family, res->ai_socktype, res->ai_protocol);
    check(tcp_fd >= 0, "socket AF_INET6");
    check(connect(tcp_fd, res->ai_addr, res->ai_addrlen) == 0, "connect AF_INET6");

    int dom = 0;
    socklen_t dlen = sizeof dom;
    check(getsockopt(tcp_fd, SOL_SOCKET, SO_DOMAIN, &dom, &dlen) == 0 && dom == AF_INET6, "SO_DOMAIN AF_INET6");

    struct sockaddr_in6 peer;
    socklen_t plen = sizeof peer;
    check(getpeername(tcp_fd, (struct sockaddr *)&peer, &plen) == 0 &&
          peer.sin6_family == AF_INET6 && ntohs(peer.sin6_port) == 7000, "getpeername AF_INET6");

    const char *msg = "posix6-tcp-ping";
    check(send(tcp_fd, msg, strlen(msg), 0) == (ssize_t)strlen(msg), "send AF_INET6");
    char buf[64] = {0};
    ssize_t rn = recv(tcp_fd, buf, sizeof buf - 1, 0);
    check(rn == 15 && memcmp(buf, "posix6-tcp-pong", 15) == 0, "recv AF_INET6");
    close(tcp_fd);
    freeaddrinfo(res);

    int udp_fd = socket(AF_INET6, SOCK_DGRAM, 0);
    check(udp_fd >= 0, "udp socket AF_INET6");
    struct sockaddr_in6 dst6 = {0};
    dst6.sin6_family = AF_INET6;
    dst6.sin6_port = htons(7001);
    check(inet_pton(AF_INET6, "2001:db8:1::90", &dst6.sin6_addr) == 1, "inet_pton");
    const char *umsg = "posix6-udp-ping";
    check(sendto(udp_fd, umsg, strlen(umsg), 0, (struct sockaddr *)&dst6, sizeof dst6) == (ssize_t)strlen(umsg),
          "sendto AF_INET6");
    struct sockaddr_in6 src6 = {0};
    socklen_t slen = sizeof src6;
    ssize_t un = recvfrom(udp_fd, buf, sizeof buf - 1, 0, (struct sockaddr *)&src6, &slen);
    check(un == 15 && memcmp(buf, "posix6-udp-pong", 15) == 0 && src6.sin6_family == AF_INET6,
          "recvfrom AF_INET6");
    close(udp_fd);

    printf("[NET-IPV6-POSIX] AF_INET6 getaddrinfo, TCP and UDP passed\n");
    printf("[NET-IPV6-POSIX] complete\n");
    fflush(stdout);
    return 0;
}

