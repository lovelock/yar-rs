/*
 * test_client_server.c - Integration test for yar client/server
 *
 * Tests the full RPC roundtrip:
 *   1. fork() a child process to run a yar_server in standalone mode
 *   2. Parent creates a yar_client and calls the server
 *   3. Verify the response
 *
 * Server setup:
 *   - Listens on 127.0.0.1:<random_port>
 *   - Standalone mode (no daemon, no pre-fork)
 *   - Registers an "echo" handler that returns a fixed string
 *   - Registers a "status" handler that returns the server status
 *
 * IMPORTANT for Rust rewrite:
 *   - The server uses libevent for async I/O
 *   - The server uses a global singleton (static yar_server *server)
 *   - yar_server_shutdown() calls event_loopexit() to stop the event loop
 *   - The wire protocol is: [header 82B] ["MSGPACK\0" 8B] [msgpack body]
 *   - Client sends: header + packager + packed request
 *   - Server responds: header + packager + packed response
 *   - Non-persistent connections close after each request
 *
 * Known issues tested here:
 *   - Due to the null-wrapping quirk in set_parameters, the echo handler
 *     cannot actually echo back client parameters. Instead it returns
 *     a fixed response string.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/time.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>
#include <errno.h>
#include <time.h>
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

/*
 * Echo handler: returns a fixed string "echo:ok"
 * (Cannot echo parameters due to set_parameters wrapping quirk)
 */
static void echo_handler(yar_request *request, yar_response *response, void *data) {
    (void)request;
    (void)data;

    yar_packager *retval = yar_pack_start_string();
    yar_pack_push_string(retval, "echo:ok", 7);

    /* Set return value directly (bypass null-wrapping) */
    if (response->out) {
        yar_pack_free((yar_packager *)response->out);
    }
    response->out = retval;
}

/*
 * Status handler: returns the string "running"
 */
static void status_handler(yar_request *request, yar_response *response, void *data) {
    (void)request;
    (void)data;

    yar_packager *retval = yar_pack_start_string();
    yar_pack_push_string(retval, "running", 7);

    if (response->out) {
        yar_pack_free((yar_packager *)response->out);
    }
    response->out = retval;
}

/* Handler table - must be null-terminated */
static yar_server_handler handlers[] = {
    {"echo",   4, echo_handler},
    {"status", 6, status_handler},
    {NULL,     0, NULL}
};

/*
 * Get a random port in range [10000, 60000]
 */
static int get_random_port(void) {
    srand(time(NULL) ^ getpid());
    return 10000 + (rand() % 50000);
}

/*
 * Run the server in the current process (called from forked child).
 */
static void run_server(const char *hostname) {
    int standalone = 1;

    if (!yar_server_init((char *)hostname)) {
        fprintf(stderr, "Server init failed\n");
        exit(1);
    }

    yar_server_set_opt(YAR_STAND_ALONE, &standalone);
    yar_server_set_opt(YAR_LOG_LEVEL, &(int){0x10}); /* suppress most logs */
    yar_server_register_handler(handlers);

    /* yar_server_run blocks until shutdown */
    yar_server_run();

    exit(0);
}

/*
 * Wait for server to be ready by attempting connections.
 */
static int wait_for_server(const char *hostname, int max_retries) {
    int i;
    for (i = 0; i < max_retries; i++) {
        yar_client *client = yar_client_init((char *)hostname);
        if (client) {
            yar_client_destroy(client);
            return 1;
        }
        usleep(100000); /* 100ms */
    }
    return 0;
}

/*
 * Test: full client-server roundtrip.
 */
static void test_server_client_roundtrip(void) {
    pid_t pid;
    int port;
    char hostname[256];

    printf("\n[test_server_client_roundtrip]\n");

    port = get_random_port();
    snprintf(hostname, sizeof(hostname), "127.0.0.1:%d", port);
    printf("  INFO: using %s\n", hostname);

    pid = fork();
    if (pid < 0) {
        printf("  FAIL: fork() failed: %s\n", strerror(errno));
        tests_run++;
        return;
    }

    if (pid == 0) {
        /* Child: run server */
        run_server(hostname);
        /* Does not return */
    }

    /* Parent: wait for server, then test */
    printf("  INFO: server pid = %d\n", pid);

    if (!wait_for_server(hostname, 30)) {
        printf("  FAIL: server did not start within timeout\n");
        tests_run++;
        kill(pid, SIGTERM);
        waitpid(pid, NULL, 0);
        return;
    }
    printf("  PASS: server is ready\n");
    tests_run++;
    tests_passed++;

    /* Test 1: Call "echo" method */
    {
        yar_client *client = yar_client_init(hostname);
        TEST_ASSERT(client != NULL, "client created");

        if (client) {
            int timeout = 5;
            yar_client_set_opt(client, YAR_CONNECT_TIMEOUT, &timeout);

            yar_response *response = client->call(client, "echo", 0, NULL);
            TEST_ASSERT(response != NULL, "echo call returned response");

            if (response) {
                TEST_ASSERT(response->status == 0, "echo response status == 0");

                const yar_data *rdata = yar_response_get_response(response);
                if (rdata) {
                    uint size = 0;
                    yar_data_type t = yar_unpack_data_type(rdata, &size);
                    TEST_ASSERT(t == YAR_DATA_STRING, "echo return type is STRING");
                    if (t == YAR_DATA_STRING) {
                        const char *str;
                        yar_unpack_data_string(rdata, &str);
                        TEST_ASSERT(size == 7 && memcmp(str, "echo:ok", 7) == 0,
                            "echo return value is 'echo:ok'");
                    }
                } else {
                    printf("  FAIL: echo return data is null\n");
                    tests_run++;
                }

                yar_response_free(response);
                free(response);
            }

            yar_client_destroy(client);
        }
    }

    /* Test 2: Call "status" method */
    {
        yar_client *client = yar_client_init(hostname);
        TEST_ASSERT(client != NULL, "client created for status call");

        if (client) {
            yar_response *response = client->call(client, "status", 0, NULL);
            TEST_ASSERT(response != NULL, "status call returned response");

            if (response) {
                TEST_ASSERT(response->status == 0, "status response status == 0");

                const yar_data *rdata = yar_response_get_response(response);
                if (rdata) {
                    uint size = 0;
                    yar_data_type t = yar_unpack_data_type(rdata, &size);
                    TEST_ASSERT(t == YAR_DATA_STRING, "status return type is STRING");
                    if (t == YAR_DATA_STRING) {
                        const char *str;
                        yar_unpack_data_string(rdata, &str);
                        TEST_ASSERT(size == 7 && memcmp(str, "running", 7) == 0,
                            "status return value is 'running'");
                    }
                }

                yar_response_free(response);
                free(response);
            }

            yar_client_destroy(client);
        }
    }

    /* Test 3: Call undefined method (should get error) */
    {
        yar_client *client = yar_client_init(hostname);
        TEST_ASSERT(client != NULL, "client created for undefined method call");

        if (client) {
            yar_response *response = client->call(client, "undefined_method", 0, NULL);
            TEST_ASSERT(response != NULL, "undefined method call returned response");

            if (response) {
                /* Server should return an error for undefined method */
                TEST_ASSERT(response->status != 0, "undefined method has non-zero status");

                const char *msg = NULL;
                uint len = 0;
                int has_error = yar_response_get_error(response, &msg, &len);
                TEST_ASSERT(has_error == 1, "undefined method has error message");
                if (has_error && len > 0) {
                    printf("  INFO: error message = '%.*s'\n", len, msg);
                }

                yar_response_free(response);
                free(response);
            }

            yar_client_destroy(client);
        }
    }

    /* Cleanup: kill server child */
    kill(pid, SIGTERM);
    waitpid(pid, NULL, 0);
    printf("  INFO: server process terminated\n");
}

int main(void) {
    /* Don't initialize logger here - the server child will init it.
     * The parent process doesn't need logging. */

    printf("=== yar_client_server integration tests ===\n");

    test_server_client_roundtrip();

    printf("\n--- Results: %d/%d passed ---\n", tests_passed, tests_run);
    return (tests_passed == tests_run) ? 0 : 1;
}
