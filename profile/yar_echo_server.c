
/**
 * yar_echo_server.c
 *
 * Minimal echo server for the yacht profiler: the response retval is the
 * request parameters tree itself, spliced verbatim via yar_pack_push_data()
 * (same shape the Rust profiler's Packager::new() + push_data produces).
 *
 * CLI (mirrors ../yar-c/example/server.c):
 *   -S <addr>   listen address (host:port), required
 *   -n <childs> number of daemon children (default 2, daemon mode only)
 *   -X          standalone: do not fork, -n is ignored (1 worker)
 *   -l <file>   log file (optional)
 *   -p <file>   pid file (optional)
 *   -t <sec>    per-request read timeout, default 3 (matches C's YAR_READ_TIMEOUT)
 *   -h          usage
 */

#include <stdio.h>   	/* for fprintf */
#include <string.h> 	/* for strlen */
#include <getopt.h> 	/* for getopt */
#include <stdlib.h>
#include <sys/types.h>

#include "yar.h"

static void echo_handler(yar_request *request, yar_response *response, void *cookie) {
	yar_packager *pk;
	const yar_data *parameters = yar_request_get_parameters(request);

	if (!parameters) {
		yar_response_set_error(response, YAR_ERROR, "%s", "echo: no request parameters");
		return;
	}

	pk = yar_pack_start(YAR_DATA_NULL, 0);
	if (!pk) {
		yar_response_set_error(response, YAR_ERROR, "%s", "echo: packager init failed");
		return;
	}

	if (!yar_pack_push_data(pk, parameters)) {
		yar_pack_free(pk);
		yar_response_set_error(response, YAR_ERROR, "%s", "echo: failed to pack parameters");
		return;
	}

	yar_response_set_retval(response, pk);
	yar_pack_free(pk);
}

static yar_server_handler echo_handlers[] = {
	{"echo", sizeof("echo") - 1, echo_handler},
	{NULL, 0, NULL}
};

int main(int argc, char **argv) {
	int opt, max_childs = 2, read_timeout = 3;
	char *hostname = NULL, *yar_pid = NULL, *log_file = NULL;
	int standalone = 0;

	while ((opt = getopt(argc, argv, "hS:n:p:Xl:t:")) != -1) {
		switch (opt) {
			case 'n':
				max_childs = atoi(optarg);
				break;
			case 'S':
				hostname = optarg;
				break;
			case 'p':
				yar_pid = optarg;
				break;
			case 'X':
				standalone = 1;
				break;
			case 'l':
				log_file = optarg;
				break;
			case 't':
				read_timeout = atoi(optarg);
				break;
			default:
				yar_server_print_usage(argv[0]);
				return 0;
		}
	}

	if (!hostname) {
		yar_server_print_usage(argv[0]);
		return 0;
	}

	if (yar_server_init(hostname)) {
		yar_server_set_opt(YAR_STAND_ALONE, &standalone);
		yar_server_set_opt(YAR_MAX_CHILDREN, &max_childs);
		yar_server_set_opt(YAR_PID_FILE, yar_pid);
		yar_server_set_opt(YAR_LOG_FILE, log_file);
		yar_server_set_opt(YAR_READ_TIMEOUT, &read_timeout);
		yar_server_register_handler(echo_handlers);
		yar_server_run();
	}
	return 0;
}

/*
 * Local variables:
 * tab-width: 4
 * c-basic-offset: 4
 * End:
 * vim600: noet sw=4 ts=4 fdm=marker
 * vim<600: sw=4 ts=4
 */
