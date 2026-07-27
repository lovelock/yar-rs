/*
 * test_protocol.c - Tests for yar_protocol (binary header render/parse)
 *
 * The yar binary protocol header is 82 bytes (packed struct):
 *   Offset  Size  Field        Notes
 *   0       4     id           Network byte order (htonl)
 *   4       2     version      NOT set by render (left as-is)
 *   6       4     magic_num    Network byte order, always 0x80DFEC60
 *   10      4     reserved     Network byte order
 *   14      32    provider     Raw bytes, only first 16 bytes copied by render
 *   46      32    token        Raw bytes, only first 16 bytes copied by render
 *   78      4     body_len     Network byte order
 *
 * IMPORTANT for Rust rewrite:
 *   - render() copies only 16 bytes of provider/token (not full 32)
 *   - render() does NOT set the version field
 *   - parse() converts magic first, checks it, then converts id/body_len/reserved
 *   - parse() does NOT convert version
 *   - Magic number: 0x80DFEC60
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <arpa/inet.h>
#include "yar.h"

static int tests_run = 0;
static int tests_passed = 0;

#define TEST_ASSERT(cond, msg) do { \
    tests_run++; \
    if (!(cond)) { \
        printf("  FAIL: %s (line %d)\n", msg, __LINE__); \
    } else { \
        tests_passed++; \
        printf("  PASS: %s\n", msg); \
    } \
} while(0)

static void hex_dump(const char *label, const unsigned char *data, size_t len) {
    size_t i;
    printf("  %s (%zu bytes): ", label, len);
    for (i = 0; i < len; i++) {
        printf("%02x", data[i]);
    }
    printf("\n");
}

/*
 * Test: render a header with known values, verify network-byte-order output.
 *
 * NOTE: yar_protocol_render copies only 16 bytes of provider/token,
 * and does NOT set the version field.
 */
static void test_protocol_render(void) {
    yar_header header;
    unsigned char *raw;

    printf("\n[test_protocol_render]\n");

    memset(&header, 0, sizeof(header));
    yar_protocol_render(&header, 1234, "TestProvider", "TestToken", 100, 0);

    raw = (unsigned char *)&header;
    hex_dump("Rendered header", raw, sizeof(header));

    /* Magic number should be in network byte order */
    TEST_ASSERT(header.magic_num == htonl(0x80DFEC60),
        "magic_num is 0x80DFEC60 in network byte order");

    /* ID should be in network byte order */
    TEST_ASSERT(header.id == htonl(1234),
        "id is 1234 in network byte order");

    /* body_len should be in network byte order */
    TEST_ASSERT(header.body_len == htonl(100),
        "body_len is 100 in network byte order");

    /* reserved should be in network byte order */
    TEST_ASSERT(header.reserved == htonl(0),
        "reserved is 0 in network byte order");

    /* Provider: only first 16 bytes are copied by render */
    TEST_ASSERT(memcmp(header.provider, "TestProvider", 12) == 0,
        "provider first 12 bytes match 'TestProvider'");

    /* Token: only first 16 bytes are copied by render */
    TEST_ASSERT(memcmp(header.token, "TestToken", 9) == 0,
        "token first 9 bytes match 'TestToken'");

    /* Verify header size is 82 bytes (packed struct) */
    printf("  INFO: sizeof(yar_header) = %zu\n", sizeof(yar_header));
    TEST_ASSERT(sizeof(yar_header) == 82,
        "sizeof(yar_header) == 82");
}

/*
 * Test: parse a header with known network-byte-order values,
 * verify host-byte-order results.
 */
static void test_protocol_parse(void) {
    yar_header header;

    printf("\n[test_protocol_parse]\n");

    memset(&header, 0, sizeof(header));
    header.magic_num = htonl(0x80DFEC60);
    header.id = htonl(5678);
    header.body_len = htonl(256);
    header.reserved = htonl(7);

    int result = yar_protocol_parse(&header);

    TEST_ASSERT(result == 1, "parse returns 1 (success)");
    TEST_ASSERT(header.magic_num == 0x80DFEC60,
        "magic_num converted to host byte order: 0x80DFEC60");
    TEST_ASSERT(header.id == 5678,
        "id converted to host byte order: 5678");
    TEST_ASSERT(header.body_len == 256,
        "body_len converted to host byte order: 256");
    TEST_ASSERT(header.reserved == 7,
        "reserved converted to host byte order: 7");
}

/*
 * Test: parse with wrong magic number returns 0.
 */
static void test_protocol_parse_bad_magic(void) {
    yar_header header;

    printf("\n[test_protocol_parse_bad_magic]\n");

    memset(&header, 0, sizeof(header));
    header.magic_num = htonl(0xDEADBEEF);  /* wrong magic */
    header.id = htonl(1);
    header.body_len = htonl(0);

    int result = yar_protocol_parse(&header);

    TEST_ASSERT(result == 0, "parse returns 0 for bad magic");

    /* Also test with completely zeroed magic */
    memset(&header, 0, sizeof(header));
    result = yar_protocol_parse(&header);
    TEST_ASSERT(result == 0, "parse returns 0 for zero magic");
}

/*
 * Test: render then parse roundtrip preserves values.
 *
 * Test vector (hex of rendered header with id=0x12345678, body_len=0x100, reserved=0x5):
 * This can be used to verify the Rust implementation produces identical bytes.
 */
static void test_protocol_roundtrip(void) {
    yar_header rendered, parsed;
    unsigned char *raw;

    printf("\n[test_protocol_roundtrip]\n");

    memset(&rendered, 0, sizeof(rendered));
    yar_protocol_render(&rendered, 0x12345678, "RoundTripProv", "RoundTripTok", 0x100, 0x5);

    raw = (unsigned char *)&rendered;
    hex_dump("Roundtrip header (test vector)", raw, sizeof(rendered));

    /* Copy and parse */
    memcpy(&parsed, &rendered, sizeof(rendered));
    int result = yar_protocol_parse(&parsed);

    TEST_ASSERT(result == 1, "roundtrip parse succeeds");
    TEST_ASSERT(parsed.id == 0x12345678, "roundtrip id matches: 0x12345678");
    TEST_ASSERT(parsed.body_len == 0x100, "roundtrip body_len matches: 0x100");
    TEST_ASSERT(parsed.reserved == 0x5, "roundtrip reserved matches: 0x5");
    TEST_ASSERT(parsed.magic_num == 0x80DFEC60, "roundtrip magic matches: 0x80DFEC60");
    TEST_ASSERT(memcmp(parsed.provider, "RoundTripProv", 13) == 0,
        "roundtrip provider matches");
    TEST_ASSERT(memcmp(parsed.token, "RoundTripTok", 12) == 0,
        "roundtrip token matches");

    /* Print field-by-field hex for Rust test vectors */
    printf("  --- Field-by-field hex (network byte order) ---\n");
    printf("  id[4]:        ");
    for (int i = 0; i < 4; i++) printf("%02x", raw[i]);
    printf("\n");
    printf("  version[2]:   ");
    for (int i = 4; i < 6; i++) printf("%02x", raw[i]);
    printf("\n");
    printf("  magic_num[4]: ");
    for (int i = 6; i < 10; i++) printf("%02x", raw[i]);
    printf("\n");
    printf("  reserved[4]:  ");
    for (int i = 10; i < 14; i++) printf("%02x", raw[i]);
    printf("\n");
    printf("  provider[32]: ");
    for (int i = 14; i < 46; i++) printf("%02x", raw[i]);
    printf("\n");
    printf("  token[32]:    ");
    for (int i = 46; i < 78; i++) printf("%02x", raw[i]);
    printf("\n");
    printf("  body_len[4]:  ");
    for (int i = 78; i < 82; i++) printf("%02x", raw[i]);
    printf("\n");
}

int main(void) {
    printf("=== yar_protocol tests ===\n");

    test_protocol_render();
    test_protocol_parse();
    test_protocol_parse_bad_magic();
    test_protocol_roundtrip();

    printf("\n--- Results: %d/%d passed ---\n", tests_passed, tests_run);
    return (tests_passed == tests_run) ? 0 : 1;
}
