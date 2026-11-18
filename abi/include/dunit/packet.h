/* Native Dunit copied-packet queue v1 (little-endian x86_64). */
#ifndef DUNIT_PACKET_H
#define DUNIT_PACKET_H

#include <stdint.h>

#define DUNIT_PACKET_VERSION 1
#define DUNIT_PACKET_MIN_FRAME_LEN 60
#define DUNIT_PACKET_MAX_FRAME_LEN 1514
#define DUNIT_PACKET_MAX_RECORDS 32
#define DUNIT_PACKET_MAX_BATCH_LEN (48u * 1024u)
#define DUNIT_PACKET_LINK_UP 1u
#define DUNIT_PACKET_RESET 1u

struct dunit_packet_options {
    uint16_t version;
    uint16_t flags;
    uint8_t reserved[12];
};

struct dunit_packet_info {
    uint16_t version;
    uint16_t flags;
    uint16_t mtu;
    uint16_t next_rx_record_len;
    uint8_t mac[6];
    uint8_t reserved[2];
    uint64_t rx_frames;
    uint64_t rx_bytes;
    uint64_t rx_dropped;
    uint64_t tx_frames;
    uint64_t tx_bytes;
    uint64_t tx_errors;
};

struct dunit_packet_control {
    uint16_t version;
    uint16_t operation;
    uint8_t reserved[12];
};

/* Followed by frame_len bytes without FCS and zero bytes to 8-byte alignment. */
struct dunit_packet_frame {
    uint16_t frame_len;
    uint16_t flags;
    uint32_t reserved;
};

#endif /* DUNIT_PACKET_H */
