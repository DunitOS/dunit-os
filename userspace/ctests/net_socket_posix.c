/* Outbound IPv4 source-compatibility checks over the stock E1000/netd path. */
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dunit/posix-sockets.h>
#include <dunit/syscall.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define SERVICE_PATH "/system/libexec/net_socket_posix"
#define DEADLINE_MS 25000
#define UDP_PORT 5433
#define TCP_PORT 5432

static const char udp_request[] = "fixture-udp-request-v1";
static const char udp_reply[] = "fixture-udp-truncated-v1";
static const char tcp_reply[] = "fixture-tcp-partial-read-and-EOF";

static void check(int condition, const char *operation)
{
    if (!condition) {
        fprintf(stderr, "[POSIX-SOCKET] FAIL %s errno=%d\n", operation, errno);
        exit(1);
    }
}

static uint64_t monotonic_ms(void)
{
    struct timespec stamp;
    check(clock_gettime(CLOCK_MONOTONIC, &stamp) == 0, "clock_gettime");
    return (uint64_t)stamp.tv_sec * 1000 + (uint64_t)stamp.tv_nsec / 1000000;
}

static int remaining_ms(uint64_t deadline)
{
    uint64_t now = monotonic_ms();
    check(now < deadline, "deadline exhausted");
    return (int)(deadline - now);
}

static void pause_briefly(uint64_t deadline)
{
    int remaining = remaining_ms(deadline);
    int delay = remaining < 20 ? remaining : 20;
    check(poll(NULL, 0, delay) == 0, "pause before retry");
}

static int wait_socket(int fd, short events, uint64_t deadline)
{
    struct pollfd request = { .fd = fd, .events = events };
    check(poll(&request, 1, remaining_ms(deadline)) == 1, "socket poll");
    check(!(request.revents & POLLNVAL), "socket poll reported invalid fd");
    return request.revents;
}

static uint32_t parse_number(const char *text)
{
    char *end;
    check(text && *text, "missing numeric value");
    errno = 0;
    unsigned long number = strtoul(text, &end, 10);
    check(errno == 0 && *end == '\0' && number > 0 && number <= UINT32_MAX,
          "invalid numeric value");
    return (uint32_t)number;
}

static uint32_t receive_grant(void)
{
    uint32_t supervisor = parse_number(getenv("DUNIT_NET_SUPERVISOR"));
    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    for (;;) {
        uint8_t payload[256];
        uint32_t sender = 0, attachment = 0;
        long size = syscall(SYS_IPC_RECEIVE_HANDLE, (long)payload, (long)sizeof payload,
                            (long)&sender, (long)&attachment, 0L, 0L);
        if (size == -1 && errno == EAGAIN) {
            pause_briefly(deadline);
            continue;
        }
        check(size >= 0, "receive supervisor packet");
        if (attachment != 0)
            check(syscall(SYS_HANDLE_CLOSE, (long)attachment, 0L, 0L, 0L, 0L, 0L) == 0,
                  "close unrelated attachment");
        if (sender == supervisor && attachment == 0 && size == 4) {
            uint32_t grant = (uint32_t)payload[0] | (uint32_t)payload[1] << 8 |
                             (uint32_t)payload[2] << 16 | (uint32_t)payload[3] << 24;
            check(grant != 0, "receive nonzero grant");
            return grant;
        }
        (void)remaining_ms(deadline);
    }
}

static struct sockaddr_in endpoint(const char *ip, uint16_t port)
{
    struct sockaddr_in address = { .sin_family = AF_INET, .sin_port = htons(port) };
    check(inet_pton(AF_INET, ip, &address.sin_addr) == 1, "inet_pton");
    return address;
}

static int same_endpoint(const struct sockaddr_in *actual, const struct sockaddr_in *expected)
{
    return actual->sin_family == expected->sin_family &&
           actual->sin_addr.s_addr == expected->sin_addr.s_addr &&
           actual->sin_port == expected->sin_port;
}

static int open_socket(int type, int protocol, uint64_t deadline)
{
    for (;;) {
        int fd = socket(AF_INET, type | SOCK_NONBLOCK | SOCK_CLOEXEC, protocol);
        if (fd >= 0) {
            check(fcntl(fd, F_GETFD) == FD_CLOEXEC, "SOCK_CLOEXEC");
            check((fcntl(fd, F_GETFL) & O_NONBLOCK) != 0, "SOCK_NONBLOCK");
            return fd;
        }
        check(errno == EPIPE || errno == EAGAIN || errno == ENOENT, "socket open");
        pause_briefly(deadline);
    }
}

static void check_no_grant(void)
{
    check(socket(AF_INET, SOCK_DGRAM | SOCK_NONBLOCK, IPPROTO_UDP) == -1 &&
          errno == EACCES, "socket without installed grant");
}

static void check_child_isolation(int fd, uint32_t grant)
{
    char fd_text[24], grant_text[24];
    snprintf(fd_text, sizeof fd_text, "%d", fd);
    snprintf(grant_text, sizeof grant_text, "%u", grant);
    char *const argv[] = { SERVICE_PATH, "child-denial", fd_text, grant_text, NULL };
    check(fcntl(fd, F_SETFD, 0) == 0, "clear CLOEXEC before inheritance check");
    pid_t pid;
    check(posix_spawn(&pid, SERVICE_PATH, NULL, NULL, argv, NULL) == 0, "spawn denied child");
    int status;
    check(waitpid(pid, &status, 0) == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0,
          "child descriptor and grant isolation");
    puts("[POSIX-SOCKET] grant and descriptor stay with owner");
}

static int exercise_udp(uint32_t grant)
{
    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    int original = open_socket(SOCK_DGRAM, IPPROTO_UDP, deadline);
    check_child_isolation(original, grant);
    int fd = dup(original);
    check(fd >= 0 && fd != original, "same-owner socket dup");
    check(close(original) == 0, "close original UDP alias");

    struct sockaddr_in peer = endpoint("192.0.2.1", UDP_PORT);
    for (;;) {
        ssize_t count = sendto(fd, udp_request, sizeof udp_request - 1, MSG_DONTWAIT,
                               (const struct sockaddr *)&peer, sizeof peer);
        if (count >= 0) {
            check((size_t)count == sizeof udp_request - 1, "UDP sendto length");
            break;
        }
        check(errno == EAGAIN, "UDP sendto retry");
        check(wait_socket(fd, POLLOUT, deadline) & POLLOUT, "UDP write readiness");
    }
    check(wait_socket(fd, POLLIN, deadline) & POLLIN, "UDP read readiness");
    int pair[2];
    check(pipe(pair) == 0, "pipe for mixed poll");
    check(write(pair[1], "x", 1) == 1, "signal pipe for mixed poll");
    struct pollfd mixed[] = {
        { .fd = fd, .events = POLLIN },
        { .fd = pair[0], .events = POLLIN },
    };
    check(poll(mixed, 2, 0) == 2 && mixed[0].revents == POLLIN &&
          mixed[1].revents == POLLIN, "poll socket and pipe together");
    check(close(pair[0]) == 0 && close(pair[1]) == 0, "close mixed poll pipe");
    char data[5];
    struct sockaddr_in source;
    socklen_t length = sizeof source;
    check(recvfrom(fd, data, sizeof data, MSG_DONTWAIT | MSG_TRUNC,
                   (struct sockaddr *)&source, &length) == (ssize_t)(sizeof udp_reply - 1),
          "UDP MSG_TRUNC original length");
    check(length == sizeof source && same_endpoint(&source, &peer) &&
          memcmp(data, udp_reply, sizeof data) == 0, "UDP source and truncated payload");
    check(recvfrom(fd, data, sizeof data, 0, NULL, NULL) == -1 && errno == EAGAIN,
          "UDP datagram consumed exactly once");
    puts("[POSIX-SOCKET] UDP sendto/recvfrom truncation and poll passed");
    return fd;
}

static void exercise_tcp(void)
{
    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    int fd = open_socket(SOCK_STREAM, IPPROTO_TCP, deadline);
    struct sockaddr_in peer = endpoint("192.0.2.1", TCP_PORT);
    check(connect(fd, (const struct sockaddr *)&peer, sizeof peer) == -1 &&
          errno == EINPROGRESS, "TCP nonblocking connect reports EINPROGRESS");
    check(wait_socket(fd, POLLOUT, deadline) & POLLOUT, "TCP writable after connect");
    int error;
    socklen_t length = sizeof error;
    check(getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &length) == 0 &&
          length == sizeof error && error == 0, "TCP SO_ERROR");
    length = sizeof error;
    check(getsockopt(fd, SOL_SOCKET, SO_RCVBUF, &error, &length) == -1 &&
          errno == ENOPROTOOPT, "unsupported socket option");
    struct sockaddr_in address;
    length = sizeof address;
    check(getpeername(fd, (struct sockaddr *)&address, &length) == 0 &&
          length == sizeof address && same_endpoint(&address, &peer), "TCP peer address");
    length = sizeof address;
    check(getsockname(fd, (struct sockaddr *)&address, &length) == 0 &&
          length == sizeof address && address.sin_family == AF_INET &&
          address.sin_addr.s_addr == endpoint("192.0.2.12", 0).sin_addr.s_addr &&
          ntohs(address.sin_port) >= 49152, "TCP local address and ephemeral port");

    unsigned char request[5000];
    for (size_t i = 0; i < sizeof request; i++)
        request[i] = i % 251;
    size_t sent = 0;
    int partial = 0;
    while (sent < sizeof request) {
        size_t remaining = sizeof request - sent;
        ssize_t count = send(fd, request + sent, remaining, MSG_NOSIGNAL);
        if (count < 0) {
            check(errno == EAGAIN, "TCP send retry");
            check(wait_socket(fd, POLLOUT, deadline) & POLLOUT, "TCP send readiness");
            continue;
        }
        check(count > 0 && (size_t)count <= remaining, "TCP send count");
        partial |= (size_t)count < remaining;
        sent += (size_t)count;
    }
    check(partial, "TCP partial send");

    size_t received = 0, chunks = 0;
    for (;;) {
        char data[7];
        ssize_t count = recv(fd, data, sizeof data, MSG_DONTWAIT);
        if (count < 0) {
            check(errno == EAGAIN, "TCP receive retry");
            check(wait_socket(fd, POLLIN, deadline) & (POLLIN | POLLHUP),
                  "TCP receive readiness");
            continue;
        }
        if (count == 0)
            break;
        check((size_t)count <= sizeof data && received + (size_t)count < sizeof tcp_reply,
              "TCP receive length");
        check(memcmp(data, tcp_reply + received, (size_t)count) == 0, "TCP stream data");
        received += (size_t)count;
        chunks++;
    }
    check(received == sizeof tcp_reply - 1 && chunks > 1, "TCP partial reads and EOF");
    while (shutdown(fd, SHUT_WR) != 0) {
        check(errno == EAGAIN, "TCP shutdown retry");
        check(wait_socket(fd, POLLOUT, deadline) & POLLOUT, "TCP shutdown readiness");
    }
    check(close(fd) == 0, "close TCP descriptor");
    puts("[POSIX-SOCKET] TCP connect, partial I/O, endpoints and EOF passed");
}

static int exercise_tcp_reset(void)
{
    uint64_t deadline = monotonic_ms() + DEADLINE_MS;
    int fd = open_socket(SOCK_STREAM, IPPROTO_TCP, deadline);
    struct sockaddr_in peer = endpoint("192.0.2.1", TCP_PORT);
    check(connect(fd, (const struct sockaddr *)&peer, sizeof peer) == -1 &&
          errno == EINPROGRESS, "TCP reset connect reports EINPROGRESS");
    check(wait_socket(fd, POLLOUT, deadline) & POLLOUT, "TCP connected before reset");
    check(send(fd, "r", 1, MSG_NOSIGNAL) == 1, "TCP reset synchronization byte");
    int flags = wait_socket(fd, POLLIN, deadline);
    check((flags & (POLLERR | POLLHUP)) == (POLLERR | POLLHUP), "TCP reset poll");
    int error;
    socklen_t length = sizeof error;
    check(getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &length) == 0 &&
          length == sizeof error && error == EPIPE, "TCP reset SO_ERROR");
    char byte;
    check(recv(fd, &byte, 1, 0) == -1 && errno == EPIPE, "TCP reset recv");
    puts("[POSIX-SOCKET] TCP reset woke poll and returned EPIPE");
    return fd;
}

int main(int argc, char **argv)
{
    check_no_grant();
    if (argc == 4 && strcmp(argv[1], "child-denial") == 0) {
        int fd = (int)parse_number(argv[2]);
        uint32_t grant = parse_number(argv[3]);
        char byte;
        check(read(fd, &byte, 1) == -1 && errno == EBADF, "child inherited socket descriptor");
        check(dunit_socket_use_grant(grant) == -1, "child inherited connect grant");
        return 0;
    }
    uint32_t grant = receive_grant();
    check(dunit_socket_use_grant(grant) == 0, "install connect grant");
    check(socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP) == -1 && errno == EOPNOTSUPP,
          "unsupported blocking socket");
    int idle = exercise_udp(grant);
    int reset = exercise_tcp_reset();
    exercise_tcp();
    check(close(reset) == 0, "close reset TCP socket");
    int flags = wait_socket(idle, POLLIN, monotonic_ms() + DEADLINE_MS);
    check((flags & (POLLERR | POLLHUP)) == (POLLERR | POLLHUP), "socket revoke poll");
    char byte;
    check(recvfrom(idle, &byte, 1, 0, NULL, NULL) == -1 && errno == EPIPE,
          "socket revoke EPIPE");
    check(close(idle) == 0, "close revoked UDP socket");
    check(syscall(SYS_HANDLE_CLOSE, (long)grant, 0L, 0L, 0L, 0L, 0L) == 0, "close connect grant");
    puts("[POSIX-SOCKET] revoked socket woke poll and returned EPIPE");
    puts("[POSIX-SOCKET] complete");
    return 0;
}
