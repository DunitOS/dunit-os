/* Native Dunit IPv4 socket and service bridge ABI v1 (little-endian x86_64). */
#ifndef DUNIT_SOCKET_H
#define DUNIT_SOCKET_H

#include <stdint.h>

#define DUNIT_SOCKET_VERSION 1u
#define DUNIT_SOCKET_PROTO_UDP 1u
#define DUNIT_SOCKET_PROTO_TCP 2u
#define DUNIT_SOCKET_SCOPE_UDP 1u
#define DUNIT_SOCKET_SCOPE_TCP 2u
#define DUNIT_SOCKET_MAX_PAYLOAD 4096u
#define DUNIT_SOCKET_MAX_FRAME 4136u

#define DUNIT_SOCKET_OP_CONNECT 1u
#define DUNIT_SOCKET_OP_SEND_TO 2u
#define DUNIT_SOCKET_OP_RECV_FROM 3u
#define DUNIT_SOCKET_OP_SHUTDOWN_WRITE 4u
#define DUNIT_SOCKET_OP_ERROR 5u
#define DUNIT_SOCKET_OP_ENDPOINTS 6u

#define DUNIT_SOCKET_BRIDGE_OPEN 1u
#define DUNIT_SOCKET_BRIDGE_CONNECT 2u
#define DUNIT_SOCKET_BRIDGE_SEND 3u
#define DUNIT_SOCKET_BRIDGE_SHUTDOWN 4u
#define DUNIT_SOCKET_BRIDGE_CLOSE 5u
#define DUNIT_SOCKET_BRIDGE_COMPLETE 6u
#define DUNIT_SOCKET_BRIDGE_DATA 7u
#define DUNIT_SOCKET_BRIDGE_EOF 8u
#define DUNIT_SOCKET_BRIDGE_ERROR 9u
#define DUNIT_SOCKET_BRIDGE_CREDIT 10u

struct dunit_socket_options {
    uint16_t version;
    uint8_t reserved[6];
};

struct dunit_socket_endpoint {
    uint8_t ipv4[4];
    uint16_t port;
    uint16_t reserved;
};

/* Payload immediately follows this header. full_len describes the datagram
 * before truncation to the output buffer capacity. */
struct dunit_socket_receive {
    struct dunit_socket_endpoint source;
    uint32_t full_len;
    uint32_t reserved;
};

struct dunit_socket_endpoints {
    struct dunit_socket_endpoint local;
    struct dunit_socket_endpoint remote;
};

/* One record per service bridge read/write, followed by exactly length bytes.
 * COMPLETE uses flags to identify the completed request; other records use 0. */
struct dunit_socket_bridge {
    uint64_t token;
    uint64_t owner;
    uint16_t kind;
    uint16_t flags;
    int32_t error;
    uint8_t ipv4[4];
    uint16_t port;
    uint16_t protocol;
    uint32_t length;
    uint32_t reserved;
};

#endif /* DUNIT_SOCKET_H */
