CC = gcc
CFLAGS = -I../legacy -I/opt/homebrew/include -g -Wall -Wno-unused-result
LDFLAGS = -L/opt/homebrew/lib -lmsgpackc -levent

# Link against the yar source files in legacy/
YAR_SRCS = ../legacy/yar_protocol.c ../legacy/yar_pack.c ../legacy/yar_request.c ../legacy/yar_response.c \
           ../legacy/yar_log.c ../legacy/yar_client.c ../legacy/yar_server.c

TESTS = test_protocol test_pack test_request test_response test_client_server

all: $(TESTS)

test_protocol: test_protocol.c ../legacy/yar_protocol.c
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS)

test_pack: test_pack.c ../legacy/yar_pack.c
	$(CC) $(CFLAGS) -o $@ $^ -L/opt/homebrew/lib -lmsgpackc -lm

test_request: test_request.c $(YAR_SRCS)
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS) -lm

test_response: test_response.c $(YAR_SRCS)
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS) -lm

test_client_server: test_client_server.c $(YAR_SRCS)
	$(CC) $(CFLAGS) -o $@ $^ $(LDFLAGS) -lm

run: all
	@echo "============================================"
	@echo "Running yar C test suite"
	@echo "============================================"
	@pass=0; fail=0; \
	for t in $(TESTS); do \
		echo ""; \
		echo "=== $$t ==="; \
		if ./$$t; then \
			pass=$$((pass + 1)); \
		else \
			fail=$$((fail + 1)); \
		fi; \
	done; \
	echo ""; \
	echo "============================================"; \
	echo "Suite: $$pass passed, $$fail failed out of $$(echo $(TESTS) | wc -w | tr -d ' ') tests"; \
	echo "============================================"; \
	test $$fail -eq 0

clean:
	rm -f $(TESTS)

.PHONY: all run clean
