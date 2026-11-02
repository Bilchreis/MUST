/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* A program that uses must's SHALL C ABI the way a C or LabVIEW program would, for
   benchmarks (see README.md). One node "bench" with one module "m": readable double
   parameters p0..p9 and value, status, and a writable double w. POSIX only.

   usage: bench_host MODE PORT [ARG...]
     cb    PORT                     callback mode; runs until killed
     poll  PORT LOOP_MS [drain|one] polling mode in a loop that sleeps LOOP_MS per pass and
                                    calls GET CMD until it returns no data (drain, default)
                                    or once (one); runs until killed
     push  PORT SECONDS             callback mode; calls SECoP_S_updateParameter2 on m:value
                                    in a tight loop for SECONDS and prints the rate
     pushv PORT SECONDS             the same with SECoP_S_updateParameter and a handle

   BENCH_POLL=<seconds> sets the module property pollinterval (0: nothing polled). */

#define _POSIX_C_SOURCE 199309L
#include "SECoP.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define NPAR 10

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec * 1e-9;
}

static void sleep_ms(int ms)
{
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (long)(ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}

/* getter of the double parameters: always 1.5 */
static void get_double(const char* name, enum SECoP_S_error* err, CSECoPbaseType** data,
                       CSECoPbaseType** sigma, double* t)
{
    (void)name;
    (void)sigma;
    (void)t;
    SECoP_V_modifyDouble(*data, 0, 0, 1.5);
    *err = SECoP_S_SUCCESS;
}

/* getter or setter that keeps the value it is given */
static void keep(const char* name, enum SECoP_S_error* err, CSECoPbaseType** data,
                 CSECoPbaseType** sigma, double* t)
{
    (void)name;
    (void)data;
    (void)sigma;
    (void)t;
    *err = SECoP_S_SUCCESS;
}

static void add_parameter(const char* name, SECoP_S_getsetFunction getter, const char* datainfo)
{
    SECoP_S_addReadableParameter(name, getter, "bench");
    SECoP_S_addPropertyString("description", name, "bench");
    SECoP_S_addPropertyJSON("datainfo", datainfo, "bench");
}

static void build(int callbacks, unsigned short port)
{
    char name[16];
    const char* poll = getenv("BENCH_POLL");
    int i;
    SECoP_S_initLibrary(NULL, 0, callbacks, "bench");
    SECoP_S_createNode("bench", "benchmark node", port, "bench");
    SECoP_S_addPropertyString("equipment_id", "bench", "bench");
    SECoP_S_addModule("m", "bench");
    SECoP_S_addPropertyString("description", "module", "bench");
    SECoP_S_addPropertyJSON("interface_classes", "[\"Readable\"]", "bench");
    if (poll != NULL)
        SECoP_S_addPropertyDouble("pollinterval", atof(poll), "bench");
    for (i = 0; i < NPAR; i++) {
        snprintf(name, sizeof name, "p%d", i);
        add_parameter(name, get_double, "{\"type\":\"double\"}");
    }
    add_parameter("value", get_double, "{\"type\":\"double\"}");
    add_parameter("status", keep,
                  "{\"type\":\"tuple\",\"members\":[{\"type\":\"enum\",\"members\":{\"IDLE\":100,\"BUSY\":300}},{\"type\":\"string\"}]}");
    SECoP_S_updateParameter2("bench:m:status", "[100,\"\"]", -1, NULL, 0, 0.0);
    SECoP_S_addWritableParameter("w", get_double, keep, "bench");
    SECoP_S_addPropertyString("description", "writable", "bench");
    SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "bench");
    i = SECoP_S_nodeComplete("bench");
    if (i < 0) {
        fprintf(stderr, "SECoP_S_nodeComplete: %d\n", i);
        SECoP_S_showErrors();
        exit(1);
    }
    fprintf(stderr, "node ready on port %u (%s)\n", (unsigned)port, callbacks ? "callbacks" : "polling");
}

/* fetches and answers one request; returns 0 when there was none */
static int handle_one(long* reads)
{
    unsigned long long id;
    enum SECoP_S_action action;
    char name[256], value[1024];
    int name_size = (int)sizeof name, value_size = (int)sizeof value;
    if (SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "bench") < 0)
        return 0;
    if (action == SECoP_S_ACTION_CHANGE)
        SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, value, value_size, NULL, 0, 0.0);
    else if (strstr(name, ":status") != NULL)
        SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, "[100,\"\"]", -1, NULL, 0, 0.0);
    else
        SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, "1.5", 3, NULL, 0, 0.0);
    if (action == SECoP_S_ACTION_READ)
        ++*reads;
    return 1;
}

static void poll_loop(int loop_ms, int drain)
{
    long reads = 0;
    double start = now_s(), last = start;
    for (;;) {
        if (drain) {
            while (handle_one(&reads)) {
            }
        } else {
            handle_one(&reads);
        }
        sleep_ms(loop_ms);
        if (now_s() - last >= 10.0) {
            fprintf(stderr, "[host] %.0f s: %ld read requests answered\n", now_s() - start, reads);
            last = now_s();
        }
    }
}

static void push_loop(double seconds, int handle)
{
    long n = 0;
    char text[64];
    double t0, t;
    CSECoPbaseType* value = SECoP_V_createSimpleDouble(0.0);
    sleep_ms(1500); /* let the clients connect and activate */
    t0 = now_s();
    do {
        int k;
        for (k = 0; k < 1000; k++, n++) {
            if (handle) {
                SECoP_V_modifyDouble(value, 0, 0, (double)n);
                SECoP_S_updateParameter("bench:m:value", value, NULL, 0.0);
            } else {
                int len = snprintf(text, sizeof text, "%ld.25", n);
                SECoP_S_updateParameter2("bench:m:value", text, len, NULL, 0, 0.0);
            }
        }
        t = now_s();
    } while (t - t0 < seconds);
    printf("updates: %ld in %.2f s = %.0f/s, %.2f us/call\n", n, t - t0, (double)n / (t - t0),
           (t - t0) / (double)n * 1e6);
    fflush(stdout);
    SECoP_V_destroy(&value);
    SECoP_S_doneLibrary(0, "bench");
}

int main(int argc, char** argv)
{
    unsigned short port;
    if (argc < 3) {
        fprintf(stderr, "usage: bench_host cb|poll|push|pushv PORT [ARG...]\n");
        return 2;
    }
    port = (unsigned short)atoi(argv[2]);
    if (strcmp(argv[1], "cb") == 0) {
        build(1, port);
        for (;;)
            sleep_ms(1000);
    } else if (strcmp(argv[1], "poll") == 0) {
        build(0, port);
        poll_loop(argc > 3 ? atoi(argv[3]) : 50, !(argc > 4 && strcmp(argv[4], "one") == 0));
    } else if (strcmp(argv[1], "push") == 0 || strcmp(argv[1], "pushv") == 0) {
        build(1, port);
        push_loop(argc > 3 ? atof(argv[3]) : 3.0, strcmp(argv[1], "pushv") == 0);
    } else {
        fprintf(stderr, "unknown mode %s\n", argv[1]);
        return 2;
    }
    return 0;
}
