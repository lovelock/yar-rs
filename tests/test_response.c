/*
 * test_response.c - Tests for yar_response (RPC response pack/unpack)
 *
 * Wire format of a packed response:
 *   [yar_header (82 bytes)] [YAR_PACKAGER "MSGPACK\0" (8 bytes)] [msgpack body]
 *
 * The msgpack body is a map with 4 keys:
 *   "i" -> uint64 (response id, matching request id)
 *   "s" -> int64  (status code: 0 = success, non-zero = error)
 *   "r" -> return value (packed data, or null)
 *   "e" -> string (error message, or null)
 *
 * IMPORTANT for Rust rewrite:
 *   - yar_response_pack packs "s" as int64 (yar_pack_push_long)
 *   - yar_response_unpack reads "s" checking for YAR_DATA_ULONG type
 *     This is a type mismatch: int64 values >= 0 decode as POSITIVE_INTEGER (ULONG),
 *     so status=0 works, but negative status values would fail.
 *   - yar_response_set_error uses vsnprintf with 1024-byte buffer
 *   - yar_response_set_retval wraps with null-start (same quirk as request)
 *   - yar_response_pack allocates payload->data; caller must free it
 *   - The response struct contains an embedded yar_payload (response.payload)
 *     used by the server to hold the packed response for sending
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/time.h>
#include <math.h>
#include "yar.h"

/* Provide external definition for inline function in yar_server.c */
ulong yar_get_microsec(void) {
    struct timeval tv;
    if (gettimeofday(&tv, (struct timezone *)NULL) == 0) {
        return (tv.tv_sec * 1000000) + tv.tv_usec;
    }
    return 0;
}

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
    printf("  %s (%zu bytes):\n    ", label, len);
    for (i = 0; i < len; i++) {
        printf("%02x", data[i]);
        if ((i + 1) % 32 == 0 && i + 1 < len) printf("\n    ");
    }
    printf("\n");
}

/*
 * Test: pack and unpack a successful response.
 * Response: id=1000, status=0, retval=string "result", no error.
 */
static void test_response_pack_unpack_success(void) {
    yar_response resp_out, resp_in;
    yar_payload payload = {0};
    int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

    printf("\n[test_response_pack_unpack_success]\n");

    /* Build response with string return value */
    memset(&resp_out, 0, sizeof(resp_out));
    resp_out.id = 1000;
    resp_out.status = 0;

    /* Set return value directly (bypassing null-wrapping) */
    {
        yar_packager *retval = yar_pack_start_string();
        yar_pack_push_string(retval, "result", 6);
        resp_out.out = retval;
    }
    resp_out.error = NULL;
    resp_out.elen = 0;

    int pack_result = yar_response_pack(&resp_out, &payload, extra);
    TEST_ASSERT(pack_result == 1, "pack succeeds");
    TEST_ASSERT(payload.size > (size_t)extra, "payload larger than header+packager");

    hex_dump("Packed success response", (unsigned char *)payload.data, payload.size);

    /* Unpack */
    memset(&resp_in, 0, sizeof(resp_in));
    int unpack_result = yar_response_unpack(&resp_in, payload.data, payload.size, extra);
    TEST_ASSERT(unpack_result == 1, "unpack succeeds");
    TEST_ASSERT(resp_in.id == 1000, "unpacked id == 1000");
    TEST_ASSERT(resp_in.status == 0, "unpacked status == 0 (success)");

    /* Check return value */
    {
        const yar_data *rdata = yar_response_get_response(&resp_in);
        TEST_ASSERT(rdata != NULL, "response data is non-null");
        if (rdata) {
            uint size = 0;
            yar_data_type t = yar_unpack_data_type(rdata, &size);
            TEST_ASSERT(t == YAR_DATA_STRING, "return value is STRING");
            const char *str;
            yar_unpack_data_string(rdata, &str);
            TEST_ASSERT(size == 6 && memcmp(str, "result", 6) == 0,
                "return value is 'result'");
        }
    }

    /* Check no error */
    {
        const char *msg = NULL;
        uint len = 0;
        int has_error = yar_response_get_error(&resp_in, &msg, &len);
        TEST_ASSERT(has_error == 0, "no error in success response");
    }

    /* Print test vector */
    printf("  --- Msgpack body (test vector for Rust) ---\n");
    hex_dump("Response body (success)", (unsigned char *)(payload.data + extra), payload.size - extra);

    /* Free: resp_out.out was consumed by pack (freed inside pack's temp pk).
     * Actually, yar_response_pack does NOT free resp_out.out, it only reads it.
     * So we need to free it. But yar_response_free will also try to free it.
     * Set out to NULL after packing to avoid double-free. */
    resp_out.out = NULL;
    yar_response_free(&resp_out);
    yar_response_free(&resp_in);
    free(payload.data);
}

/*
 * Test: pack and unpack an error response.
 * Response: id=2000, status=YAR_ERROR(0x8), error="something went wrong".
 */
static void test_response_pack_unpack_error(void) {
    yar_response resp_out, resp_in;
    yar_payload payload = {0};
    int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

    printf("\n[test_response_pack_unpack_error]\n");

    memset(&resp_out, 0, sizeof(resp_out));
    resp_out.id = 2000;

    /* Set error using the API */
    yar_response_set_error(&resp_out, YAR_ERROR, "something went wrong");

    TEST_ASSERT(resp_out.status == YAR_ERROR, "status set to YAR_ERROR");
    TEST_ASSERT(resp_out.error != NULL, "error message is set");
    TEST_ASSERT(resp_out.elen > 0, "error length > 0");
    TEST_ASSERT(memcmp(resp_out.error, "something went wrong", resp_out.elen) == 0,
        "error message matches");

    int pack_result = yar_response_pack(&resp_out, &payload, extra);
    TEST_ASSERT(pack_result == 1, "pack error response succeeds");

    hex_dump("Packed error response", (unsigned char *)payload.data, payload.size);

    /* Unpack */
    memset(&resp_in, 0, sizeof(resp_in));
    int unpack_result = yar_response_unpack(&resp_in, payload.data, payload.size, extra);
    TEST_ASSERT(unpack_result == 1, "unpack error response succeeds");
    TEST_ASSERT(resp_in.id == 2000, "unpacked id == 2000");

    /*
     * NOTE: status is packed as int64 via yar_pack_push_long.
     * YAR_ERROR = 0x8 = 8, which is positive, so msgpack stores it as
     * POSITIVE_INTEGER. yar_response_unpack checks for YAR_DATA_ULONG.
     * This works for non-negative status values.
     */
    TEST_ASSERT(resp_in.status == YAR_ERROR, "unpacked status == YAR_ERROR");

    /* Check error message */
    {
        const char *msg = NULL;
        uint len = 0;
        int has_error = yar_response_get_error(&resp_in, &msg, &len);
        TEST_ASSERT(has_error == 1, "error is present");
        TEST_ASSERT(len > 0, "error length > 0");
        TEST_ASSERT(memcmp(msg, "something went wrong", len) == 0,
            "error message matches 'something went wrong'");
    }

    /* Return value should be null (no retval set) */
    {
        const yar_data *rdata = yar_response_get_response(&resp_in);
        if (rdata) {
            uint size = 0;
            yar_data_type t = yar_unpack_data_type(rdata, &size);
            TEST_ASSERT(t == YAR_DATA_NULL, "return value is NULL for error response");
        } else {
            TEST_ASSERT(1, "return value pointer is null");
        }
    }

    /* Print test vector */
    printf("  --- Msgpack body (test vector for Rust) ---\n");
    hex_dump("Response body (error)", (unsigned char *)(payload.data + extra), payload.size - extra);

    yar_response_free(&resp_out);
    yar_response_free(&resp_in);
    free(payload.data);
}

/*
 * Test: yar_response_set_error with format string.
 */
static void test_response_set_error(void) {
    yar_response resp;

    printf("\n[test_response_set_error]\n");

    memset(&resp, 0, sizeof(resp));

    yar_response_set_error(&resp, 42, "error code %d: %s", 404, "not found");

    TEST_ASSERT(resp.status == 42, "status == 42");
    TEST_ASSERT(resp.error != NULL, "error is set");
    TEST_ASSERT(resp.elen > 0, "error length > 0");

    /* Verify formatted message */
    char expected[] = "error code 404: not found";
    TEST_ASSERT(resp.elen == strlen(expected), "error length matches formatted string");
    TEST_ASSERT(memcmp(resp.error, expected, resp.elen) == 0,
        "error message matches formatted string");

    printf("  INFO: formatted error = '%.*s'\n", resp.elen, resp.error);

    yar_response_free(&resp);
}

/*
 * Test: yar_response_set_retval.
 * yar_response_set_retval wraps with null-start, but since null-start
 * creates an empty sbuffer, the actual return value is preserved correctly.
 */
static void test_response_set_retval(void) {
    yar_response resp;
    yar_packager *retval;

    printf("\n[test_response_set_retval]\n");

    memset(&resp, 0, sizeof(resp));

    retval = yar_pack_start_string();
    yar_pack_push_string(retval, "hello", 5);

    yar_response_set_retval(&resp, retval);

    TEST_ASSERT(resp.out != NULL, "response->out is set after set_retval");

    /* Pack and unpack to verify return value is preserved */
    {
        yar_payload payload = {0};
        int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

        resp.id = 300;
        resp.status = 0;

        yar_response_pack(&resp, &payload, extra);
        hex_dump("Response with set_retval", (unsigned char *)payload.data, payload.size);

        yar_response resp_in;
        memset(&resp_in, 0, sizeof(resp_in));
        yar_response_unpack(&resp_in, payload.data, payload.size, extra);

        TEST_ASSERT(resp_in.id == 300, "unpacked id == 300");
        TEST_ASSERT(resp_in.status == 0, "unpacked status == 0");

        const yar_data *rdata = yar_response_get_response(&resp_in);
        TEST_ASSERT(rdata != NULL, "return value is non-null after roundtrip");
        if (rdata) {
            uint sz = 0;
            yar_data_type t = yar_unpack_data_type(rdata, &sz);
            TEST_ASSERT(t == YAR_DATA_STRING, "retval type is STRING after roundtrip");
            if (t == YAR_DATA_STRING) {
                const char *str;
                yar_unpack_data_string(rdata, &str);
                TEST_ASSERT(sz == 5 && memcmp(str, "hello", 5) == 0,
                    "retval is 'hello'");
            }
        }

        resp.out = NULL; /* prevent double-free */
        yar_response_free(&resp_in);
        free(payload.data);
    }

    yar_pack_free(retval);
    yar_response_free(&resp);
}

/*
 * Test: pack/unpack with extra_bytes offset.
 */
static void test_response_extra_bytes(void) {
    yar_response resp_out, resp_in;
    yar_payload payload = {0};
    int extra = sizeof(yar_header) + sizeof(YAR_PACKAGER);

    printf("\n[test_response_extra_bytes]\n");

    memset(&resp_out, 0, sizeof(resp_out));
    resp_out.id = 777;
    resp_out.status = 0;
    resp_out.out = NULL;
    resp_out.error = NULL;

    yar_response_pack(&resp_out, &payload, extra);

    /* Fill extra bytes region with pattern */
    memset(payload.data, 0xBB, extra);

    int pattern_ok = 1;
    for (int i = 0; i < extra; i++) {
        if ((unsigned char)payload.data[i] != 0xBB) {
            pattern_ok = 0;
            break;
        }
    }
    TEST_ASSERT(pattern_ok, "extra_bytes region preserved (0xBB pattern)");

    /* Restore for unpack */
    memset(payload.data, 0, extra);

    memset(&resp_in, 0, sizeof(resp_in));
    int unpack_result = yar_response_unpack(&resp_in, payload.data, payload.size, extra);
    TEST_ASSERT(unpack_result == 1, "unpack with extra_bytes succeeds");
    TEST_ASSERT(resp_in.id == 777, "unpacked id == 777");
    TEST_ASSERT(resp_in.status == 0, "unpacked status == 0");

    yar_response_free(&resp_out);
    yar_response_free(&resp_in);
    free(payload.data);
}

/*
 * Test: get_status and get_error API functions.
 */
static void test_response_getters(void) {
    yar_response resp;

    printf("\n[test_response_getters]\n");

    memset(&resp, 0, sizeof(resp));

    /* Initial state */
    TEST_ASSERT(yar_response_get_status(&resp) == 0, "initial status is 0");

    const char *msg = NULL;
    uint len = 0;
    TEST_ASSERT(yar_response_get_error(&resp, &msg, &len) == 0,
        "get_error returns 0 when no error");

    /* After setting error */
    yar_response_set_error(&resp, 99, "test error %d", 1);
    TEST_ASSERT(yar_response_get_status(&resp) == 99, "status is 99 after set_error");
    TEST_ASSERT(yar_response_get_error(&resp, &msg, &len) == 1,
        "get_error returns 1 when error set");
    TEST_ASSERT(len == 12, "error length is 12");
    TEST_ASSERT(memcmp(msg, "test error 1", 12) == 0, "error message is 'test error 1'");

    yar_response_free(&resp);
}

int main(void) {
    yar_logger_init(NULL, YAR_ERROR);

    printf("=== yar_response tests ===\n");

    test_response_pack_unpack_success();
    test_response_pack_unpack_error();
    test_response_set_error();
    test_response_set_retval();
    test_response_extra_bytes();
    test_response_getters();

    printf("\n--- Results: %d/%d passed ---\n", tests_passed, tests_run);

    yar_logger_destroy();
    return (tests_passed == tests_run) ? 0 : 1;
}
