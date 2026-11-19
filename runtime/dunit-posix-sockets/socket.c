/* Opt-in nonblocking IPv4 socket translation for Dunit's musl sysroot. */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <dunit/posix-sockets.h>
#include <dunit/rights.h>
#include <dunit/socket.h>
#include <dunit/syscall.h>

#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <poll.h>
#include <string.h>
#include <unistd.h>

static uint32_t selected_grant;
enum { OPEN_TIMEOUT_MS = 5000 };

static long native_call(long number, uintptr_t first, uintptr_t second, uintptr_t third,
                        uintptr_t fourth, uintptr_t fifth, uintptr_t sixth)
{
    register uintptr_t fourth_register __asm__("r10") = fourth;
    register uintptr_t fifth_register __asm__("r8") = fifth;
    register uintptr_t sixth_register __asm__("r9") = sixth;
    long result;
    __asm__ volatile(
        "syscall"
        : "=a"(result)
        : "0"(number), "D"(first), "S"(second), "d"(third),
          "r"(fourth_register), "r"(fifth_register), "r"(sixth_register)
        : "rcx", "r11", "memory");
    return result;
}

static long fail(int error)
{
    errno = error;
    return -1;
}

static long result(long value)
{
    if (value >= 0)
        return value;
    return fail(value < -INT_MAX ? EIO : (int)-value);
}

static long socket_op(int fd, unsigned operation, const void *input, size_t input_size,
                      void *output, size_t output_size)
{
    long value = native_call(SYS_NET_SOCKET_OP, fd, operation, (uintptr_t)input,
                             input_size, (uintptr_t)output, output_size);
    /* The native operation returns EBADF for an FD of an unrelated type. */
    if (value == -EBADF && fcntl(fd, F_GETFD) >= 0)
        return -ENOTSOCK;
    return value;
}

static long query_endpoints(int fd, struct dunit_socket_endpoints *endpoints)
{
    long value = socket_op(fd, DUNIT_SOCKET_OP_ENDPOINTS, NULL, 0,
                           endpoints, sizeof *endpoints);
    if (value >= 0 && value != sizeof *endpoints)
        return -EIO;
    return value;
}

static int endpoint_error(const struct sockaddr *address, socklen_t length,
                          struct dunit_socket_endpoint *endpoint)
{
    if (!address)
        return EFAULT;
    if (length < sizeof(struct sockaddr_in))
        return EINVAL;
    struct sockaddr_in ipv4;
    memcpy(&ipv4, address, sizeof ipv4);
    if (ipv4.sin_family != AF_INET)
        return EAFNOSUPPORT;
    memcpy(endpoint->ipv4, &ipv4.sin_addr.s_addr, sizeof endpoint->ipv4);
    endpoint->port = ntohs(ipv4.sin_port);
    endpoint->reserved = 0;
    return 0;
}

static void put_address(const struct dunit_socket_endpoint *endpoint,
                        struct sockaddr *address, socklen_t *length)
{
    struct sockaddr_in ipv4 = {0};
    ipv4.sin_family = AF_INET;
    ipv4.sin_port = htons(endpoint->port);
    memcpy(&ipv4.sin_addr.s_addr, endpoint->ipv4, sizeof endpoint->ipv4);
    size_t copied = *length < sizeof ipv4 ? *length : sizeof ipv4;
    memcpy(address, &ipv4, copied);
    *length = sizeof ipv4;
}

int dunit_socket_use_grant(uint32_t grant)
{
    if (grant != 0) {
        long rights = native_call(SYS_HANDLE_RIGHTS, grant, 0, 0, 0, 0, 0);
        if (rights < 0)
            return (int)result(rights);
        if (!(rights & DUNIT_RIGHT_NET_CONNECT))
            return (int)fail(EACCES);
    }
    __atomic_store_n(&selected_grant, grant, __ATOMIC_RELEASE);
    return 0;
}

static int await_open(int fd)
{
    /* The proxy's OPEN completion must precede connect(). Otherwise its
     * POLLOUT could be mistaken for completion of a connect still unqueued.
     * The bound applies only to creation. Socket I/O stays nonblocking. */
    struct pollfd interest = { .fd = fd, .events = POLLOUT };
    int ready = poll(&interest, 1, OPEN_TIMEOUT_MS);
    if (ready < 0)
        return -1;
    if (!ready)
        return (int)fail(ETIMEDOUT);
    if (interest.revents & (POLLERR | POLLHUP | POLLNVAL)) {
        int32_t error = 0;
        long value = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &error, sizeof error);
        if (value < 0)
            return (int)result(value);
        if (value != sizeof error || error >= 0)
            return (int)fail(EIO);
        return (int)result(error);
    }
    if (!(interest.revents & POLLOUT))
        return (int)fail(EIO);
    return 0;
}

int socket(int domain, int type, int protocol)
{
    if (domain != AF_INET)
        return (int)fail(EAFNOSUPPORT);
    unsigned native_protocol;
    switch (type & ~(SOCK_NONBLOCK | SOCK_CLOEXEC)) {
    case SOCK_STREAM:
        if (protocol && protocol != IPPROTO_TCP)
            return (int)fail(EPROTONOSUPPORT);
        native_protocol = DUNIT_SOCKET_PROTO_TCP;
        break;
    case SOCK_DGRAM:
        if (protocol && protocol != IPPROTO_UDP)
            return (int)fail(EPROTONOSUPPORT);
        native_protocol = DUNIT_SOCKET_PROTO_UDP;
        break;
    default:
        return (int)fail(EPROTONOSUPPORT);
    }
    if (!(type & SOCK_NONBLOCK))
        return (int)fail(EOPNOTSUPP);
    uint32_t grant = __atomic_load_n(&selected_grant, __ATOMIC_ACQUIRE);
    if (!grant)
        return (int)fail(EACCES);
    const struct dunit_socket_options options = { .version = DUNIT_SOCKET_VERSION };
    long value = native_call(SYS_NET_SOCKET_OPEN, grant, native_protocol, (uintptr_t)&options,
                             sizeof options, 0, 0);
    if (value < 0)
        return (int)result(value);
    int fd = (int)value;
    if (await_open(fd) == 0 &&
        ((type & SOCK_CLOEXEC) || fcntl(fd, F_SETFD, 0) == 0))
        return fd;
    int error = errno;
    close(fd);
    return (int)fail(error);
}

int connect(int fd, const struct sockaddr *address, socklen_t length)
{
    struct dunit_socket_endpoint endpoint;
    int error = endpoint_error(address, length, &endpoint);
    if (error)
        return (int)fail(error);
    long value = socket_op(fd, DUNIT_SOCKET_OP_CONNECT, &endpoint, sizeof endpoint, NULL, 0);
    return (int)result(value == -EAGAIN ? -EINPROGRESS : value);
}

ssize_t send(int fd, const void *buffer, size_t length, int flags)
{
    if (flags & ~(MSG_DONTWAIT | MSG_NOSIGNAL))
        return fail(EOPNOTSUPP);
    if (length && !buffer)
        return fail(EFAULT);
    struct dunit_socket_endpoints endpoints;
    long value = query_endpoints(fd, &endpoints);
    if (value < 0)
        return result(value);
    if (!endpoints.remote.port)
        return fail(ENOTCONN);
    if (!length) {
        /* Native write of zero bytes is a no-op. Use SEND_TO to publish
         * an empty UDP datagram. A stream has no empty record to publish. */
        value = socket_op(fd, DUNIT_SOCKET_OP_SEND_TO, &endpoints.remote,
                          sizeof endpoints.remote, NULL, 0);
        return result(value == -EOPNOTSUPP ? 0 : value);
    }
    return result(native_call(SYS_WRITE, fd, (uintptr_t)buffer, length, 0, 0, 0));
}

ssize_t sendto(int fd, const void *buffer, size_t length, int flags,
               const struct sockaddr *address, socklen_t address_length)
{
    if (!address && !address_length)
        return send(fd, buffer, length, flags);
    if (flags & ~(MSG_DONTWAIT | MSG_NOSIGNAL))
        return fail(EOPNOTSUPP);
    if (length > DUNIT_SOCKET_MAX_PAYLOAD)
        return fail(EMSGSIZE);
    if (length && !buffer)
        return fail(EFAULT);
    struct {
        struct dunit_socket_endpoint endpoint;
        unsigned char payload[DUNIT_SOCKET_MAX_PAYLOAD];
    } record;
    int error = endpoint_error(address, address_length, &record.endpoint);
    if (error)
        return fail(error);
    if (length)
        memcpy(record.payload, buffer, length);
    return result(socket_op(fd, DUNIT_SOCKET_OP_SEND_TO, &record,
                            sizeof record.endpoint + length, NULL, 0));
}

ssize_t recvfrom(int fd, void *buffer, size_t length, int flags,
                 struct sockaddr *address, socklen_t *address_length)
{
    if (flags & ~(MSG_DONTWAIT | MSG_TRUNC))
        return fail(EOPNOTSUPP);
    if ((length && !buffer) || (address && !address_length))
        return fail(EFAULT);
    size_t capacity = length < DUNIT_SOCKET_MAX_PAYLOAD ? length : DUNIT_SOCKET_MAX_PAYLOAD;
    struct {
        struct dunit_socket_receive header;
        unsigned char payload[DUNIT_SOCKET_MAX_PAYLOAD];
    } record;
    long value = socket_op(fd, DUNIT_SOCKET_OP_RECV_FROM, NULL, 0, &record,
                           sizeof record.header + capacity);
    if (value == -EOPNOTSUPP) {
        /* Native RECV_FROM is datagram-only. It has already type-checked
         * the FD, so the stream fallback cannot read an unrelated file. */
        if (flags & MSG_TRUNC)
            return fail(EOPNOTSUPP);
        struct dunit_socket_endpoints endpoints;
        if (address) {
            value = query_endpoints(fd, &endpoints);
            if (value < 0)
                return result(value);
            if (!endpoints.remote.port)
                return fail(ENOTCONN);
        }
        value = native_call(SYS_READ, fd, (uintptr_t)buffer, capacity, 0, 0, 0);
        if (value >= 0 && address)
            put_address(&endpoints.remote, address, address_length);
        return result(value);
    }
    if (value < 0)
        return result(value);
    if ((size_t)value < sizeof record.header ||
        (size_t)value > sizeof record.header + capacity)
        return fail(EIO);
    size_t copied = (size_t)value - sizeof record.header;
    if (copied > record.header.full_len || record.header.full_len > DUNIT_SOCKET_MAX_PAYLOAD)
        return fail(EIO);
    if (copied)
        memcpy(buffer, record.payload, copied);
    if (address)
        put_address(&record.header.source, address, address_length);
    return flags & MSG_TRUNC ? (ssize_t)record.header.full_len : (ssize_t)copied;
}

ssize_t recv(int fd, void *buffer, size_t length, int flags)
{
    return recvfrom(fd, buffer, length, flags, NULL, NULL);
}

int shutdown(int fd, int how)
{
    if (how != SHUT_WR) {
        if (how != SHUT_RD && how != SHUT_RDWR)
            return (int)fail(EINVAL);
        return (int)fail(EOPNOTSUPP);
    }
    return (int)result(socket_op(fd, DUNIT_SOCKET_OP_SHUTDOWN_WRITE, NULL, 0, NULL, 0));
}

static int socket_name(int fd, struct sockaddr *address, socklen_t *length, int remote)
{
    if (!address || !length)
        return (int)fail(EFAULT);
    struct dunit_socket_endpoints endpoints;
    long value = query_endpoints(fd, &endpoints);
    if (value < 0)
        return (int)result(value);
    if (remote && !endpoints.remote.port)
        return (int)fail(ENOTCONN);
    put_address(remote ? &endpoints.remote : &endpoints.local, address, length);
    return 0;
}

int getsockname(int fd, struct sockaddr *address, socklen_t *length)
{
    return socket_name(fd, address, length, 0);
}

int getpeername(int fd, struct sockaddr *address, socklen_t *length)
{
    return socket_name(fd, address, length, 1);
}

int getsockopt(int fd, int level, int option, void *data, socklen_t *length)
{
    if (level != SOL_SOCKET || option != SO_ERROR)
        return (int)fail(ENOPROTOOPT);
    if (!data || !length)
        return (int)fail(EFAULT);
    if (*length < sizeof(int))
        return (int)fail(EINVAL);
    int32_t native_error = 0;
    long value = socket_op(fd, DUNIT_SOCKET_OP_ERROR, NULL, 0, &native_error, sizeof native_error);
    if (value < 0)
        return (int)result(value);
    if (value != sizeof native_error || native_error > 0 || native_error == INT32_MIN)
        return (int)fail(EIO);
    int error = -native_error;
    memcpy(data, &error, sizeof error);
    *length = sizeof error;
    return 0;
}

int setsockopt(int fd, int level, int option, const void *data, socklen_t length)
{
    (void)level;
    (void)option;
    (void)data;
    (void)length;
    struct dunit_socket_endpoints endpoints;
    long value = query_endpoints(fd, &endpoints);
    return (int)result(value < 0 ? value : -ENOPROTOOPT);
}
