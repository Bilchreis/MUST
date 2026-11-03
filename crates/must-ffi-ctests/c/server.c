/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* C conformance tests of the SECoP_S_* functions. Each test returns 0 on success or the
   line of the first failed check. The Rust side (tests/server.rs) runs them one at a
   time, as the library state is global, and talks to the nodes over TCP. */

#if !defined(_WIN32)
#define _POSIX_C_SOURCE 199309L
#endif

#include "SECoP.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
static void sleep_ms(int ms)
{
    Sleep((DWORD)ms);
}
#else
#include <time.h>
static void sleep_ms(int ms)
{
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (long)(ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}
#endif

#define CHECK(cond)                                                              \
    do {                                                                         \
        if (!(cond)) {                                                           \
            fprintf(stderr, "server.c:%d: check failed: %s\n", __LINE__, #cond); \
            return __LINE__;                                                     \
        }                                                                        \
    } while (0)

#define OK(call) CHECK((call) == SECoP_S_SUCCESS)

/* SECoP_S_nodeComplete, printing the problems if it fails */
static enum SECoP_S_error complete(const char* context)
{
    enum SECoP_S_error rc = SECoP_S_nodeComplete(context);
    if (rc < 0) {
        fprintf(stderr, "SECoP_S_nodeComplete(\"%s\"): %s\n", context, SECoP_S_getErrorString(rc));
        SECoP_S_showErrors();
    }
    return rc;
}

/* --- library, contexts, error strings */

int ctest_server_basics(void)
{
    const char* text;

    /* nothing works before SECoP_S_initLibrary */
    CHECK(SECoP_S_createNode("n1", "a node", 0, "basics") == SECoP_S_ERROR_NOT_INITIALIZED);
    CHECK(SECoP_S_addModule("m", "basics") == SECoP_S_ERROR_NOT_INITIALIZED);
    CHECK(SECoP_S_setAddFocus("n1:m") == SECoP_S_ERROR_NOT_INITIALIZED);
    CHECK(SECoP_S_updateParameter2("n1:m:p", "1", -1, NULL, 0, 0.0) == SECoP_S_ERROR_NOT_INITIALIZED);

    /* static descriptions */
    text = SECoP_S_getErrorString(SECoP_S_ERROR_READONLY);
    CHECK(text != NULL && strstr(text, "READONLY") != NULL);
    CHECK(strstr(SECoP_S_getErrorString(SECoP_S_SUCCESS), "SUCCESS") != NULL);
    CHECK(strstr(SECoP_S_getErrorString((enum SECoP_S_error)-1000), "unknown") != NULL);

    /* Qt-related arguments are accepted; the status display is requested */
    SECoP_S_setManyThreads(1);
    OK(SECoP_S_initLibrary(NULL, 1, 1, "basics"));
    SECoP_S_showStatusWindow(1);
    SECoP_S_showStatusWindow(0);

    /* node names and interfaces */
    CHECK(SECoP_S_createNode2("bad name", "x", "127.0.0.1", 0, "basics") == SECoP_S_ERROR_MISSING_MANDATORY);
    CHECK(SECoP_S_createNode2(NULL, "x", "127.0.0.1", 0, "basics") == SECoP_S_ERROR_MISSING_MANDATORY);
    CHECK(SECoP_S_createNode2("n1", "", "127.0.0.1", 0, "basics") == SECoP_S_WARNING_NO_DESCRIPTION);
    CHECK(SECoP_S_createNode2("N1", "x", "127.0.0.1", 0, "basics") == SECoP_S_ERROR_NAME_ALREADY_USED);
    /* as in SHALL, an interface that is no address means all interfaces */
    OK(SECoP_S_createNode2("n2", "x", "not-an-address", 0, "basics"));
    OK(SECoP_S_deleteNode("n2"));
    OK(SECoP_S_setAddFocus("n1:")); /* deleting the context's last node cleared its focus */

    /* building needs a known context and a focused module */
    CHECK(SECoP_S_addModule("m", "unknown") == SECoP_S_ERROR_INVALID_CONTEXT_ID);
    CHECK(SECoP_S_addReadableParameter("p", NULL, "basics") == SECoP_S_ERROR_INVALID_MODULE);
    OK(SECoP_S_addModule("m", "basics"));
    CHECK(SECoP_S_addModule("M", "basics") == SECoP_S_ERROR_NAME_ALREADY_USED);
    CHECK(SECoP_S_addModule("1m", "basics") == SECoP_S_ERROR_INVALID_NAME);
    CHECK(SECoP_S_setAddFocus("nonode:m") == SECoP_S_ERROR_INVALID_NODE);
    CHECK(SECoP_S_setAddFocus("n1:nomodule") == SECoP_S_ERROR_INVALID_MODULE);
    CHECK(SECoP_S_setAddFocus("n1:m:noparam") == SECoP_S_ERROR_INVALID_PARAMETER);
    OK(SECoP_S_setAddFocus("n1:m"));
    CHECK(SECoP_S_addPropertyDouble("pollinterval", NAN, "basics") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(SECoP_S_addPropertyJSON("_x", "{not json", "basics") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(SECoP_S_addPropertyString("_x", NULL, "basics") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(SECoP_S_addProperty("_x", NULL, "basics") == SECoP_S_ERROR_INVALID_VALUE);
    SECoP_S_showErrors();

    OK(SECoP_S_deleteNode("n1"));
    CHECK(SECoP_S_deleteNode("n1") == SECoP_S_ERROR_INVALID_NODE);
    CHECK(SECoP_S_addModule("m2", "basics") == SECoP_S_ERROR_INVALID_NODE);

    /* initializing a context again starts it afresh, as in SHALL */
    OK(SECoP_S_createNode2("n3", "x", "127.0.0.1", 0, "basics"));
    OK(SECoP_S_initLibrary(NULL, 0, 1, "basics"));
    CHECK(SECoP_S_deleteNode("n3") == SECoP_S_ERROR_INVALID_NODE);

    /* done with the only context: back to the uninitialized state */
    SECoP_S_doneLibrary(0, "basics");
    CHECK(SECoP_S_createNode("n1", "a node", 0, "basics") == SECoP_S_ERROR_NOT_INITIALIZED);
    return 0;
}

int ctest_server_done(const char* context)
{
    SECoP_S_doneLibrary(0, context);
    return 0;
}

int ctest_server_done_nodes(const char* context)
{
    SECoP_S_doneLibrary(1, context);
    return 0;
}

/* --- building */

/* SHALL's polling example from SECoP.h, with a context ID */
int ctest_server_example(unsigned short port)
{
    OK(SECoP_S_initLibrary(NULL, 1, 0, "example"));
    OK(SECoP_S_createNode("my_node", "best SEC node ever", port, "example"));
    OK(SECoP_S_addModule("mod1", "example"));
    OK(SECoP_S_addPropertyString("description", "the first module", "example"));
    OK(SECoP_S_addReadableParameter("value", 0, "example"));
    OK(SECoP_S_addPropertyString("description", "actual value", "example"));
    OK(SECoP_S_addPropertyString("datainfo", "{\"type\":\"double\", \"unit\":\"mm\"}", "example"));
    CHECK(complete("example") >= 0);
    return 0;
}

/* two contexts with interleaved calls */
int ctest_server_two_contexts(unsigned short port1, unsigned short port2)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "id1"));
    OK(SECoP_S_initLibrary(NULL, 0, 0, "id2"));
    OK(SECoP_S_createNode2("node1", "first node", "127.0.0.1", port1, "id1"));
    OK(SECoP_S_createNode2("node2", "second node", "127.0.0.1", port2, "id2"));
    OK(SECoP_S_addModule("m1", "id1"));
    OK(SECoP_S_addModule("m2", "id2"));
    OK(SECoP_S_addPropertyString("description", "module of id2", "id2"));
    OK(SECoP_S_addPropertyString("description", "module of id1", "id1"));
    OK(SECoP_S_addReadableParameter("value", NULL, "id2"));
    OK(SECoP_S_addReadableParameter("value", NULL, "id1"));
    OK(SECoP_S_addPropertyString("description", "value of id1", "id1"));
    OK(SECoP_S_addPropertyString("description", "value of id2", "id2"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"int\",\"min\":0,\"max\":10}", "id2"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "id1"));
    /* the focus of a node is selected by name, whichever context asks */
    OK(SECoP_S_setAddFocus("node1:m1"));
    OK(SECoP_S_addPropertyString("_tag", "one", "id1"));
    CHECK(complete("id2") >= 0);
    CHECK(complete("id1") >= 0);
    return 0;
}

/* builds a node with 20 modules; run by two threads at the same time */
int ctest_server_build_many(const char* context, const char* node, unsigned short port)
{
    char name[32];
    int i;
    OK(SECoP_S_initLibrary(NULL, 0, 0, context));
    OK(SECoP_S_createNode2(node, context, "127.0.0.1", port, context));
    for (i = 0; i < 20; ++i) {
        sprintf(name, "m%d", i);
        OK(SECoP_S_addModule(name, context));
        OK(SECoP_S_addPropertyString("description", context, context));
        OK(SECoP_S_addReadableParameter("value", NULL, context));
        OK(SECoP_S_addPropertyString("description", context, context));
        OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", context));
    }
    CHECK(complete(context) >= 0);
    return 0;
}

/* a parameter without datainfo: the node is not started */
int ctest_server_validation_fails(unsigned short port)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "val"));
    OK(SECoP_S_createNode2("vnode", "validation", "127.0.0.1", port, "val"));
    OK(SECoP_S_addModule("m", "val"));
    CHECK(SECoP_S_addPropertyString("myprop", "x", "val") == SECoP_S_WARNING_CUSTOM_PROPERTY);
    CHECK(SECoP_S_addPropertyString("description", "", "val") == SECoP_S_ERROR_INVALID_VALUE);
    OK(SECoP_S_addPropertyInteger("visibility", 2, "val"));
    OK(SECoP_S_addPropertyJSON("interface_classes", "[\"Readable\"]", "val"));
    OK(SECoP_S_addReadableParameter("value", NULL, "val"));
    OK(SECoP_S_addPropertyString("description", "the value", "val"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "val"));
    CHECK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "val") == SECoP_S_ERROR_NAME_ALREADY_USED);
    OK(SECoP_S_addPropertyBoolean("_flag", 1, "val"));
    OK(SECoP_S_addReadableParameter("status", NULL, "val"));
    OK(SECoP_S_addPropertyString("description", "the status", "val"));
    OK(SECoP_S_addPropertyJSON(
        "datainfo",
        "{\"type\":\"tuple\",\"members\":[{\"type\":\"enum\",\"members\":{\"IDLE\":100,\"ERROR\":400}},{\"type\":\"string\"}]}",
        "val"));
    OK(SECoP_S_addReadableParameter("nodatainfo", NULL, "val"));
    OK(SECoP_S_addPropertyString("description", "no datainfo", "val"));
    CHECK(SECoP_S_nodeComplete("val") == SECoP_S_ERROR_MISSING_MANDATORY);
    SECoP_S_showErrors();
    return 0;
}

/* fixes the node of ctest_server_validation_fails */
int ctest_server_validation_fixed(void)
{
    OK(SECoP_S_setAddFocus("vnode:m"));
    OK(SECoP_S_addPropertyString("description", "now with description", "val"));
    OK(SECoP_S_setAddFocus("vnode:m:nodatainfo"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"string\"}", "val"));
    CHECK(complete("val") >= 0);
    /* a complete node cannot change */
    CHECK(SECoP_S_addModule("late", "val") == SECoP_S_ERROR_INVALID_NODE);
    return 0;
}

/* --- callback mode */

static double g_target = 0.0;
static long long g_count = 0;

static void get_value(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                      CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)timestamp;
    if (strcmp(name, "cbnode:mod1:value") != 0) {
        *piError = SECoP_S_ERROR_INTERNAL;
        return;
    }
    SECoP_V_modifyDouble(*ppData, 0, 0, 4.2);
    if (ppSigma != NULL)
        *ppSigma = SECoP_V_createSimpleDouble(0.1);
    *piError = SECoP_S_SUCCESS;
}

static void get_target(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                       CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)ppSigma;
    SECoP_V_modifyDouble(*ppData, 0, 0, g_target);
    *timestamp = 1000.5;
    *piError = SECoP_S_SUCCESS;
}

static void set_target(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                       CSECoPbaseType** ppSigma, double* timestamp)
{
    double value = 0.0;
    (void)name;
    (void)ppSigma;
    (void)timestamp;
    if (!SECoP_V_getDouble(*ppData, 0, 0, &value)) {
        *piError = SECoP_S_ERROR_INVALID_VALUE;
    } else if (value > 100.0) {
        *piError = SECoP_S_ERROR_IS_BUSY;
    } else {
        g_target = value;
        *piError = SECoP_S_SUCCESS;
    }
}

static void get_slow(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                     CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)ppSigma;
    (void)timestamp;
    sleep_ms(1000);
    SECoP_V_modifyDouble(*ppData, 0, 0, 1.0);
    *piError = SECoP_S_SUCCESS;
}

static void get_count(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                      CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)ppSigma;
    (void)timestamp;
    SECoP_V_modifyInteger(*ppData, 0, 0, ++g_count);
    *piError = SECoP_S_SUCCESS;
}

static void call_reset(const char* name, const CSECoPbaseType* pArgument, enum SECoP_S_error* piError,
                       CSECoPbaseType** ppReturn, double* timestamp)
{
    (void)pArgument;
    (void)ppReturn;
    (void)timestamp;
    g_target = 0.0;
    *piError = strcmp(name, "cbnode:mod1:reset") == 0 ? SECoP_S_SUCCESS : SECoP_S_ERROR_INTERNAL;
}

static void call_plus1(const char* name, const CSECoPbaseType* pArgument, enum SECoP_S_error* piError,
                       CSECoPbaseType** ppReturn, double* timestamp)
{
    double value = 0.0;
    (void)name;
    (void)timestamp;
    if (!SECoP_V_getDouble(pArgument, 0, 0, &value)) {
        *piError = SECoP_S_ERROR_INVALID_VALUE;
        return;
    }
    /* replace the handle: the library frees both */
    *ppReturn = SECoP_V_createSimpleDouble(value + 1.0);
    *piError = SECoP_S_SUCCESS;
}

int ctest_server_callbacks(unsigned short port)
{
    g_target = 0.0;
    g_count = 0;
    OK(SECoP_S_initLibrary(NULL, 0, 1, "cb"));
    OK(SECoP_S_createNode2("cbnode", "callbacks", "127.0.0.1", port, "cb"));

    OK(SECoP_S_addModule("mod1", "cb"));
    OK(SECoP_S_addPropertyString("description", "callback module", "cb"));
    OK(SECoP_S_addReadableParameter("value", get_value, "cb"));
    OK(SECoP_S_addPropertyString("description", "value", "cb"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\",\"unit\":\"K\"}", "cb"));
    OK(SECoP_S_addWritableParameter("target", get_target, set_target, "cb"));
    OK(SECoP_S_addPropertyString("description", "target", "cb"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "cb"));
    OK(SECoP_S_addCommand("reset", call_reset, "cb"));
    OK(SECoP_S_addPropertyString("description", "resets the target", "cb"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"command\"}", "cb"));
    OK(SECoP_S_addCommand("plus1", call_plus1, "cb"));
    OK(SECoP_S_addPropertyString("description", "adds one", "cb"));
    OK(SECoP_S_addPropertyJSON(
        "datainfo", "{\"type\":\"command\",\"argument\":{\"type\":\"double\"},\"result\":{\"type\":\"double\"}}", "cb"));

    OK(SECoP_S_addModule("slow", "cb"));
    OK(SECoP_S_addPropertyString("description", "slow hardware", "cb"));
    OK(SECoP_S_addReadableParameter("value", get_slow, "cb"));
    OK(SECoP_S_addPropertyString("description", "value", "cb"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "cb"));

    OK(SECoP_S_addModule("counter", "cb"));
    OK(SECoP_S_addPropertyString("description", "polled module", "cb"));
    OK(SECoP_S_addPropertyDouble("pollinterval", 0.2, "cb"));
    OK(SECoP_S_addReadableParameter("value", get_count, "cb"));
    OK(SECoP_S_addPropertyString("description", "number of reads", "cb"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"int\",\"min\":0,\"max\":1000000000}", "cb"));

    CHECK(complete("cb") >= 0);
    return 0;
}

/* --- one module at a time in callback mode: the getter of mod1:a waits to be released */

static volatile int g_cb_released = 0;
static volatile int g_cb_a = 0, g_cb_b = 0, g_cb_c = 0, g_cb_target = 0;

static void get_cb(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                   CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)ppSigma;
    (void)timestamp;
    if (strcmp(name, "cbc:mod1:a") == 0) {
        int i;
        ++g_cb_a;
        for (i = 0; i < 1000 && !g_cb_released; ++i)
            sleep_ms(5);
    } else if (strcmp(name, "cbc:mod1:b") == 0) {
        ++g_cb_b;
    } else {
        ++g_cb_c;
    }
    SECoP_V_modifyDouble(*ppData, 0, 0, 2.5);
    *piError = SECoP_S_SUCCESS;
}

static enum SECoP_S_error add_cb_param(const char* name)
{
    enum SECoP_S_error rc = SECoP_S_addReadableParameter(name, get_cb, "cbc");
    if (rc < 0)
        return rc;
    rc = SECoP_S_addPropertyString("description", name, "cbc");
    if (rc < 0)
        return rc;
    return SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "cbc");
}

int ctest_cb_build(unsigned short port)
{
    g_cb_released = 0;
    g_cb_a = g_cb_b = g_cb_c = 0;
    OK(SECoP_S_initLibrary(NULL, 0, 1, "cbc"));
    OK(SECoP_S_createNode2("cbc", "one module at a time", "127.0.0.1", port, "cbc"));
    OK(SECoP_S_addModule("mod1", "cbc"));
    OK(SECoP_S_addPropertyString("description", "module", "cbc"));
    OK(SECoP_S_addPropertyDouble("pollinterval", 0.0, "cbc"));
    OK(add_cb_param("a"));
    OK(add_cb_param("b"));
    OK(SECoP_S_addModule("mod2", "cbc"));
    OK(SECoP_S_addPropertyString("description", "another module", "cbc"));
    OK(SECoP_S_addPropertyDouble("pollinterval", 0.0, "cbc"));
    OK(add_cb_param("c"));
    CHECK(complete("cbc") >= 0);
    return 0;
}

static void set_cb(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                   CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)ppData;
    (void)ppSigma;
    (void)timestamp;
    ++g_cb_target;
    *piError = SECoP_S_SUCCESS;
}

/* the same with a short node timeout, so the getter of mod1:a outlives it, and a setter */
int ctest_cb_hang_build(unsigned short port, double pollinterval)
{
    g_cb_released = 0;
    g_cb_a = g_cb_b = g_cb_c = g_cb_target = 0;
    OK(SECoP_S_initLibrary(NULL, 0, 1, "cbc"));
    OK(SECoP_S_createNode2("cbc", "calls outliving the timeout", "127.0.0.1", port, "cbc"));
    OK(SECoP_S_addPropertyDouble("timeout", 0.3, "cbc"));
    OK(SECoP_S_addModule("mod1", "cbc"));
    OK(SECoP_S_addPropertyString("description", "module", "cbc"));
    OK(SECoP_S_addPropertyDouble("pollinterval", pollinterval, "cbc"));
    OK(add_cb_param("a"));
    OK(add_cb_param("b"));
    OK(SECoP_S_addWritableParameter("target", get_cb, set_cb, "cbc"));
    OK(SECoP_S_addPropertyString("description", "target", "cbc"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "cbc"));
    OK(SECoP_S_addModule("mod2", "cbc"));
    OK(SECoP_S_addPropertyString("description", "another module", "cbc"));
    OK(SECoP_S_addPropertyDouble("pollinterval", 0.0, "cbc"));
    OK(add_cb_param("c"));
    CHECK(complete("cbc") >= 0);
    return 0;
}

int ctest_cb_calls(const char* parameter)
{
    switch (parameter[0]) {
    case 'a':
        return g_cb_a;
    case 'b':
        return g_cb_b;
    case 't':
        return g_cb_target;
    default:
        return g_cb_c;
    }
}

void ctest_cb_release(void)
{
    g_cb_released = 1;
}

/* --- polling in callback mode: SHALL's pollinterval keeps one interval for all */

/* reads per parameter of the "drv" module, and setter calls of its pollinterval */
static int g_drv_value, g_drv_status, g_drv_target, g_drv_ramp, g_drv_interval_sets;
static double g_drv_interval = 0.0;

int ctest_server_poll_count(const char* parameter)
{
    if (strcmp(parameter, "value") == 0)
        return g_drv_value;
    if (strcmp(parameter, "status") == 0)
        return g_drv_status;
    if (strcmp(parameter, "target") == 0)
        return g_drv_target;
    if (strcmp(parameter, "ramp") == 0)
        return g_drv_ramp;
    if (strcmp(parameter, "pollinterval_sets") == 0)
        return g_drv_interval_sets;
    return -1;
}

static void get_drv(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                    CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)ppSigma;
    (void)timestamp;
    *piError = SECoP_S_SUCCESS;
    if (strcmp(name, "pnode:drv:value") == 0) {
        ++g_drv_value;
        SECoP_V_modifyDouble(*ppData, 0, 0, 4.2);
    } else if (strcmp(name, "pnode:drv:status") == 0) {
        ++g_drv_status; /* the default status value (DISABLED, "") is fine */
    } else if (strcmp(name, "pnode:drv:target") == 0) {
        ++g_drv_target;
    } else if (strcmp(name, "pnode:drv:ramp") == 0) {
        ++g_drv_ramp;
    } else if (strcmp(name, "pnode:drv:pollinterval") == 0) {
        SECoP_V_modifyDouble(*ppData, 0, 0, g_drv_interval);
    } else {
        *piError = SECoP_S_ERROR_INTERNAL;
    }
}

static void set_drv(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                    CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)ppSigma;
    (void)timestamp;
    *piError = SECoP_S_SUCCESS;
    if (strcmp(name, "pnode:drv:pollinterval") == 0) {
        ++g_drv_interval_sets;
        SECoP_V_getDouble(*ppData, 0, 0, &g_drv_interval);
    }
}

static void call_drv(const char* name, const CSECoPbaseType* pArgument, enum SECoP_S_error* piError,
                     CSECoPbaseType** ppReturn, double* timestamp)
{
    (void)name;
    (void)pArgument;
    (void)ppReturn;
    (void)timestamp;
    *piError = SECoP_S_SUCCESS;
}

static enum SECoP_S_error add_drv_param(const char* name, int writable, const char* datainfo)
{
    enum SECoP_S_error rc = writable ? SECoP_S_addWritableParameter(name, get_drv, set_drv, "poll")
                                     : SECoP_S_addReadableParameter(name, get_drv, "poll");
    if (rc != SECoP_S_SUCCESS)
        return rc;
    rc = SECoP_S_addPropertyString("description", name, "poll");
    if (rc != SECoP_S_SUCCESS)
        return rc;
    return SECoP_S_addPropertyJSON("datainfo", datainfo, "poll");
}

/* A Drivable module "drv" (value, status, target, ramp) whose poll configuration is
   chosen by `mode`:
   0: SHALL's module property pollinterval 0.2 s, ramp's parameter property 1 s
   1: as 0, plus a writable pollinterval parameter (the properties are then ignored)
   2: only ramp's parameter property 1 s
   3: nothing */
int ctest_server_poll_config(unsigned short port, int mode)
{
    enum SECoP_S_error rc;
    const char* kelvin = "{\"type\":\"double\",\"unit\":\"K\"}";
    g_drv_value = g_drv_status = g_drv_target = g_drv_ramp = g_drv_interval_sets = 0;
    g_drv_interval = 0.0;
    OK(SECoP_S_initLibrary(NULL, 0, 1, "poll"));
    OK(SECoP_S_createNode2("pnode", "polling", "127.0.0.1", port, "poll"));
    OK(SECoP_S_addModule("drv", "poll"));
    OK(SECoP_S_addPropertyString("description", "polled drivable", "poll"));
    OK(SECoP_S_addPropertyJSON("interface_classes", "[\"Drivable\"]", "poll"));
    if (mode == 0 || mode == 1)
        OK(SECoP_S_addPropertyDouble("pollinterval", 0.2, "poll"));
    OK(add_drv_param("value", 0, kelvin));
    OK(add_drv_param("status", 0,
                     "{\"type\":\"tuple\",\"members\":[{\"type\":\"enum\",\"members\":"
                     "{\"DISABLED\":0,\"IDLE\":100,\"BUSY\":300}},{\"type\":\"string\"}]}"));
    OK(add_drv_param("target", 1, kelvin));
    OK(add_drv_param("ramp", 1, kelvin));
    if (mode <= 2)
        OK(SECoP_S_addPropertyDouble("pollinterval", 1.0, "poll"));
    if (mode == 1)
        OK(add_drv_param("pollinterval", 1, "{\"type\":\"double\",\"min\":0,\"unit\":\"s\"}"));
    OK(SECoP_S_addCommand("stop", call_drv, "poll"));
    OK(SECoP_S_addPropertyString("description", "stop", "poll"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"command\"}", "poll"));
    rc = complete("poll");
    CHECK(rc >= 0);
    /* the ignored properties are reported as warnings */
    CHECK(mode != 1 || rc > 0);
    return 0;
}

/* Polling mode without poll configuration: a Readable module is polled by default, so
   the host gets stored read requests for value and status about every 5 s. Answers
   them for 5.8 s and checks that each was requested once after the startup reads. */
int ctest_server_poll_default_requests(unsigned short port)
{
    int ms = 0, value_reads = 0, status_reads = 0;
    OK(SECoP_S_initLibrary(NULL, 0, 0, "pdef"));
    OK(SECoP_S_createNode2("pdnode", "default polling", "127.0.0.1", port, "pdef"));
    OK(SECoP_S_addModule("sensor", "pdef"));
    OK(SECoP_S_addPropertyString("description", "polled by default", "pdef"));
    OK(SECoP_S_addReadableParameter("value", NULL, "pdef"));
    OK(SECoP_S_addPropertyString("description", "value", "pdef"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "pdef"));
    OK(SECoP_S_addReadableParameter("status", NULL, "pdef"));
    OK(SECoP_S_addPropertyString("description", "status", "pdef"));
    OK(SECoP_S_addPropertyJSON("datainfo",
                               "{\"type\":\"tuple\",\"members\":[{\"type\":\"enum\",\"members\":"
                               "{\"IDLE\":100}},{\"type\":\"string\"}]}",
                               "pdef"));
    CHECK(complete("pdef") >= 0);
    for (ms = 0; ms < 5800; ms += 10) {
        unsigned long long id = 0;
        enum SECoP_S_action action = SECoP_S_ACTION_NONE;
        char name[64], value[64];
        int name_size = (int)sizeof(name), value_size = (int)sizeof(value);
        enum SECoP_S_error rc =
            SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "pdef");
        if (rc == SECoP_S_ERROR_NO_DATA) {
            sleep_ms(10);
            continue;
        }
        OK(rc);
        CHECK(action == SECoP_S_ACTION_READ);
        if (strcmp(name, "pdnode:sensor:value") == 0) {
            value_reads += ms > 1000;
            OK(SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, "1.0", -1, NULL, 0, 0.0));
        } else {
            CHECK(strcmp(name, "pdnode:sensor:status") == 0);
            status_reads += ms > 1000;
            OK(SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, "[100,\"ok\"]", -1, NULL, 0, 0.0));
        }
    }
    CHECK(value_reads == 1);
    CHECK(status_reads == 1);
    SECoP_S_doneLibrary(0, "pdef");
    return 0;
}

/* in callback mode every parameter needs its functions */
int ctest_server_missing_functions(void)
{
    OK(SECoP_S_initLibrary(NULL, 0, 1, "nofunc"));
    OK(SECoP_S_createNode2("nfnode", "missing functions", "127.0.0.1", 0, "nofunc"));
    OK(SECoP_S_addModule("m", "nofunc"));
    OK(SECoP_S_addPropertyString("description", "module", "nofunc"));
    OK(SECoP_S_addWritableParameter("target", get_target, NULL, "nofunc"));
    OK(SECoP_S_addPropertyString("description", "target", "nofunc"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "nofunc"));
    CHECK(SECoP_S_nodeComplete("nofunc") == SECoP_S_ERROR_NO_SETTER);
    SECoP_S_showErrors();
    SECoP_S_doneLibrary(0, "nofunc");
    return 0;
}

/* --- polling mode */

int ctest_server_polling(unsigned short port)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "poll"));
    OK(SECoP_S_createNode2("pnode", "polling", "127.0.0.1", port, "poll"));
    OK(SECoP_S_addModule("mod1", "poll"));
    OK(SECoP_S_addPropertyString("description", "polled module", "poll"));
    OK(SECoP_S_addReadableParameter("value", NULL, "poll"));
    OK(SECoP_S_addPropertyString("description", "value", "poll"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "poll"));
    OK(SECoP_S_addWritableParameter("target", NULL, NULL, "poll"));
    OK(SECoP_S_addPropertyString("description", "target", "poll"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "poll"));
    OK(SECoP_S_addReadableParameter("limit", NULL, "poll"));
    OK(SECoP_S_addPropertyString("description", "limit", "poll"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "poll"));
    OK(SECoP_S_addPropertyJSON("constant", "10.0", "poll"));
    OK(SECoP_S_addCommand("go", NULL, "poll"));
    OK(SECoP_S_addPropertyString("description", "go", "poll"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"command\",\"argument\":{\"type\":\"int\",\"min\":0,\"max\":10}}", "poll"));
    /* a value pushed before the node is complete is its initial value */
    OK(SECoP_S_updateParameter2("pnode:mod1:value", "1.5", -1, NULL, 0, NAN));
    CHECK(complete("poll") >= 0);
    CHECK(SECoP_S_getStoredCommand2(NULL, NULL, NULL, NULL, NULL, NULL, "nocontext") == SECoP_S_ERROR_INVALID_CONTEXT_ID);
    return 0;
}

/* waits for the next request of context "poll" with getStoredCommand2 */
static enum SECoP_S_error poll2(unsigned long long* id, enum SECoP_S_action* action, char* name, int* name_size,
                                char* value, int* value_size)
{
    int i;
    enum SECoP_S_error rc = SECoP_S_ERROR_NO_DATA;
    int name_cap = *name_size, value_cap = *value_size;
    for (i = 0; i < 500 && rc == SECoP_S_ERROR_NO_DATA; ++i) {
        *name_size = name_cap;
        *value_size = value_cap;
        rc = SECoP_S_getStoredCommand2(id, action, name, name_size, value, value_size, "poll");
        if (rc == SECoP_S_ERROR_NO_DATA)
            sleep_ms(10);
    }
    return rc;
}

static unsigned long long g_late_id = 0;

/* one step of the polling test; the Rust side sends the matching request before */
int ctest_server_poll_step(int step)
{
    unsigned long long id = 0;
    enum SECoP_S_action action = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int name_size = (int)sizeof(name), value_size = (int)sizeof(value);
    CSECoPbaseType* handle = NULL;
    long long argument = 0;

    switch (step) {
    case 1: /* change mod1:target 500 */
        OK(poll2(&id, &action, name, &name_size, value, &value_size));
        CHECK(id != 0 && action == SECoP_S_ACTION_CHANGE);
        /* as in SHALL, the size returned is the length without NUL */
        CHECK(strcmp(name, "pnode:mod1:target") == 0 && name_size == (int)strlen(name));
        CHECK(strtod(value, NULL) == 500.0);
        CHECK(SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "poll") == SECoP_S_ERROR_NO_DATA);
        OK(SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, value, (int)strlen(value), NULL, 0, 0.0));
        /* each request is answered once */
        CHECK(SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, value, (int)strlen(value), NULL, 0, 0.0) < 0);
        return 0;

    case 2: /* read mod1:value, with a too small name buffer: handed out anyway, as in SHALL */
        name_size = 4;
        CHECK(poll2(&id, &action, name, &name_size, value, &value_size) == SECoP_S_WARNING_BUFFER_TOO_SMALL);
        CHECK(strcmp(name, "pno") == 0 && name_size == (int)strlen("pnode:mod1:value"));
        CHECK(action == SECoP_S_ACTION_READ && value[0] == '\0' && value_size == 0);
        CHECK(SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "poll") == SECoP_S_ERROR_NO_DATA);
        OK(SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, "2.5", -1, "0.25", -1, 1000.5));
        return 0;

    case 3: /* do mod1:go 3, with the handle variant */
    {
        int i;
        enum SECoP_S_error rc = SECoP_S_ERROR_NO_DATA;
        for (i = 0; i < 500 && rc == SECoP_S_ERROR_NO_DATA; ++i) {
            name_size = (int)sizeof(name);
            rc = SECoP_S_getStoredCommand(&id, &action, name, &name_size, &handle, "poll");
            if (rc == SECoP_S_ERROR_NO_DATA)
                sleep_ms(10);
        }
        OK(rc);
        CHECK(action == SECoP_S_ACTION_DO && strcmp(name, "pnode:mod1:go") == 0);
        CHECK(handle != NULL && SECoP_V_getInteger(handle, 0, 0, &argument) && argument == 3);
        SECoP_V_destroy(&handle);
        OK(SECoP_S_putCommandAnswer(id, SECoP_S_SUCCESS, NULL, NULL, 0.0));
        return 0;
    }

    case 4: /* change mod1:target 7: the hardware is busy */
        OK(poll2(&id, &action, name, &name_size, value, &value_size));
        OK(SECoP_S_putCommandAnswer2(id, SECoP_S_ERROR_IS_BUSY, NULL, 0, NULL, 0, 0.0));
        return 0;

    case 5: /* read mod1:value, which will not be answered in time */
    {
        int i;
        enum SECoP_S_error rc = SECoP_S_ERROR_NO_DATA;
        handle = SECoP_V_createSimpleDouble(0.0); /* overwritten */
        for (i = 0; i < 500 && rc == SECoP_S_ERROR_NO_DATA; ++i) {
            name_size = (int)sizeof(name);
            rc = SECoP_S_getStoredCommand(&id, &action, name, &name_size, &handle, "poll");
            if (rc == SECoP_S_ERROR_NO_DATA)
                sleep_ms(10);
        }
        OK(rc);
        /* a read has no value */
        CHECK(action == SECoP_S_ACTION_READ && handle == NULL);
        g_late_id = id;
        return 0;
    }

    case 6: /* the late answer */
        /* SHALL's code for an unknown id */
        CHECK(SECoP_S_putCommandAnswer2(g_late_id, SECoP_S_SUCCESS, "1.0", -1, NULL, 0, 0.0) == SECoP_S_ERROR_INVALID_NAME);
        CHECK(SECoP_S_putCommandAnswer2(g_late_id + 1000, SECoP_S_SUCCESS, "1.0", -1, NULL, 0, 0.0) == SECoP_S_ERROR_INVALID_NAME);
        return 0;

    case 7: /* pushed updates */
        OK(SECoP_S_updateParameter2("pnode:mod1:value", "3.5", 3, NULL, 0, NAN));
        handle = SECoP_V_createSimpleDouble(4.5);
        {
            CSECoPbaseType* sigma = SECoP_V_createSimpleDouble(0.5);
            OK(SECoP_S_updateParameter("PNODE:mod1:value", handle, sigma, 1234.5));
            SECoP_V_destroy(&sigma);
        }
        SECoP_V_destroy(&handle);
        CHECK(SECoP_S_updateParameter2("pnode:mod1:nope", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_PARAMETER);
        CHECK(SECoP_S_updateParameter2("pnode:mod2:value", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_MODULE);
        CHECK(SECoP_S_updateParameter2("other:mod1:value", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_NODE);
        CHECK(SECoP_S_updateParameter2("pnode:mod1:value", "\"text\"", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_VALUE);
        CHECK(SECoP_S_updateParameter2("pnode:mod1:go", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_PARAMETER);
        /* SHALL's codes for a constant and for names missing a part */
        CHECK(SECoP_S_updateParameter2("pnode:mod1:limit", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_PARAMETER);
        CHECK(SECoP_S_updateParameter2("", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_NAME);
        CHECK(SECoP_S_updateParameter2("pnode", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_NODE);
        CHECK(SECoP_S_updateParameter2("pnode:mod1", "1", -1, NULL, 0, NAN) == SECoP_S_ERROR_INVALID_MODULE);
        /* as in SHALL, the text ends at the first NUL: a size counting the NUL or a whole
           buffer works */
        OK(SECoP_S_updateParameter2("pnode:mod1:value", "5.5", 4, NULL, 0, NAN));
        OK(SECoP_S_updateParameter2("pnode:mod1:value", "6.5\0junk", 8, "0.5\0junk", 8, NAN));
        return 0;

    case 8: /* read mod1:value, answered with sizes that count the NUL and what follows */
        OK(poll2(&id, &action, name, &name_size, value, &value_size));
        CHECK(action == SECoP_S_ACTION_READ);
        OK(SECoP_S_putCommandAnswer2(id, SECoP_S_SUCCESS, "7.5\0junk", 8, "0.75", 5, 0.0));
        return 0;

    default:
        return __LINE__;
    }
}

/* --- several requests of one module at once, in polling mode: the Rust side sends
   requests, these functions play the host */

static struct {
    char name[64];
    unsigned long long id;
} g_fetched[16];
static int g_fetched_count = 0;

static enum SECoP_S_error add_conc_param(const char* name, int writable)
{
    enum SECoP_S_error rc = writable ? SECoP_S_addWritableParameter(name, NULL, NULL, "conc")
                                     : SECoP_S_addReadableParameter(name, NULL, "conc");
    if (rc < 0)
        return rc;
    rc = SECoP_S_addPropertyString("description", name, "conc");
    if (rc < 0)
        return rc;
    return SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "conc");
}

/* kind 0: mod1 with p0 to p3 and target, not polled;
   kind 1: mod1 with value, polled every 0.2 s;
   kind 2: mod1 with value, p0 and target, polled every 0.2 s */
int ctest_conc_build(unsigned short port, int kind)
{
    g_fetched_count = 0;
    OK(SECoP_S_initLibrary(NULL, 0, 0, "conc"));
    OK(SECoP_S_createNode2("cn", "concurrent requests", "127.0.0.1", port, "conc"));
    OK(SECoP_S_addModule("mod1", "conc"));
    OK(SECoP_S_addPropertyString("description", "polled module", "conc"));
    OK(SECoP_S_addPropertyDouble("pollinterval", kind == 0 ? 0.0 : 0.2, "conc"));
    if (kind == 0) {
        OK(add_conc_param("p0", 0));
        OK(add_conc_param("p1", 0));
        OK(add_conc_param("p2", 0));
        OK(add_conc_param("p3", 0));
    }
    if (kind != 0)
        OK(add_conc_param("value", 0));
    if (kind == 2)
        OK(add_conc_param("p0", 0));
    if (kind != 1)
        OK(add_conc_param("target", 1));
    CHECK(complete("conc") >= 0);
    return 0;
}

/* fetches the next request, without answering any: of the given name unless NULL */
int ctest_conc_fetch(const char* expected, int action)
{
    unsigned long long id = 0;
    enum SECoP_S_action got = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int i;
    enum SECoP_S_error rc = SECoP_S_ERROR_NO_DATA;
    for (i = 0; i < 500 && rc == SECoP_S_ERROR_NO_DATA; ++i) {
        int name_size = (int)sizeof(name), value_size = (int)sizeof(value);
        rc = SECoP_S_getStoredCommand2(&id, &got, name, &name_size, value, &value_size, "conc");
        if (rc == SECoP_S_ERROR_NO_DATA)
            sleep_ms(10);
    }
    OK(rc);
    CHECK((int)got == action);
    CHECK(expected == NULL || strcmp(name, expected) == 0);
    CHECK(g_fetched_count < 16);
    strcpy(g_fetched[g_fetched_count].name, name);
    g_fetched[g_fetched_count].id = id;
    ++g_fetched_count;
    return 0;
}

/* no request is waiting */
int ctest_conc_none(void)
{
    unsigned long long id = 0;
    enum SECoP_S_action action = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int name_size = (int)sizeof(name), value_size = (int)sizeof(value);
    CHECK(SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "conc") == SECoP_S_ERROR_NO_DATA);
    return 0;
}

/* answers the request of that name fetched last */
int ctest_conc_answer(const char* name, const char* value)
{
    int i;
    for (i = g_fetched_count - 1; i >= 0; --i) {
        if (g_fetched[i].id != 0 && strcmp(g_fetched[i].name, name) == 0) {
            OK(SECoP_S_putCommandAnswer2(g_fetched[i].id, SECoP_S_SUCCESS, value, -1, NULL, 0, 0.0));
            g_fetched[i].id = 0;
            return 0;
        }
    }
    return __LINE__;
}

/* a host that sets a short node timeout and never fetches a request: step 0 builds the
   node, step 1 runs after the client got its TimeoutError */
int ctest_server_abandoned(int step, unsigned short port)
{
    unsigned long long id = 0;
    enum SECoP_S_action action = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int name_size = (int)sizeof(name), value_size = (int)sizeof(value);

    if (step == 0) {
        OK(SECoP_S_initLibrary(NULL, 0, 0, "abandon"));
        OK(SECoP_S_createNode2("anode", "abandoned requests", "127.0.0.1", port, "abandon"));
        OK(SECoP_S_addPropertyDouble("timeout", 0.5, "abandon"));
        OK(SECoP_S_addModule("mod1", "abandon"));
        OK(SECoP_S_addPropertyString("description", "module", "abandon"));
        OK(SECoP_S_addPropertyDouble("pollinterval", 0.0, "abandon"));
        OK(SECoP_S_addReadableParameter("value", NULL, "abandon"));
        OK(SECoP_S_addPropertyString("description", "value", "abandon"));
        OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "abandon"));
        CHECK(complete("abandon") >= 0);
        return 0;
    }
    /* nobody waits for the request any more: it is not handed out */
    CHECK(SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "abandon") == SECoP_S_ERROR_NO_DATA);
    return 0;
}

/* --- user error codes (must addition): the Rust side sends the requests */

/* of context "uerr": _Overheated and HardwareError; of "uerr_cb": _CbFault and another
   _Overheated */
static enum SECoP_S_error g_user_over, g_user_hw, g_user_cb, g_user_other;

static void get_user_error(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                           CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)ppData;
    (void)ppSigma;
    (void)timestamp;
    *piError = g_user_cb;
}

/* a module mod1 with the double parameter "name", not polled */
static enum SECoP_S_error add_user_module(const char* name, int writable, SECoP_S_getsetFunction get,
                                          const char* context)
{
    enum SECoP_S_error rc = SECoP_S_addModule("mod1", context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyString("description", "module", context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyDouble("pollinterval", 0.0, context);
    if (rc >= 0)
        rc = writable ? SECoP_S_addWritableParameter(name, get, NULL, context)
                      : SECoP_S_addReadableParameter(name, get, context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyString("description", name, context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", context);
    return rc;
}

/* a polling-mode node "un" (context "uerr") with mod1:target, a callback node "ucb"
   (context "uerr_cb") whose getter of mod1:value returns a user error, and the user
   errors */
int ctest_user_error_build(unsigned short poll_port, unsigned short cb_port)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "uerr"));
    OK(SECoP_S_createNode2("un", "user errors, polling", "127.0.0.1", poll_port, "uerr"));
    OK(add_user_module("target", 1, NULL, "uerr"));
    CHECK(complete("uerr") >= 0);
    OK(SECoP_S_initLibrary(NULL, 0, 1, "uerr_cb"));
    OK(SECoP_S_createNode2("ucb", "user errors, callbacks", "127.0.0.1", cb_port, "uerr_cb"));
    OK(add_user_module("value", 0, get_user_error, "uerr_cb"));
    CHECK(complete("uerr_cb") >= 0);

    CHECK(SECoP_S_registerUserError("_Overheated", "coil too hot", "uerr_none") == SECoP_S_ERROR_INVALID_CONTEXT_ID);
    g_user_over = SECoP_S_registerUserError("_Overheated", "coil too hot", "uerr");
    CHECK(g_user_over <= -1000);
    CHECK(strcmp(SECoP_S_getErrorString(g_user_over), "_Overheated: coil too hot") == 0);
    g_user_hw = SECoP_S_registerUserError("HardwareError", "sensor disconnected", "uerr");
    g_user_cb = SECoP_S_registerUserError("_CbFault", "from a callback", "uerr_cb");
    CHECK(g_user_hw <= -1000 && g_user_cb <= -1000);
    CHECK(g_user_hw != g_user_over && g_user_cb != g_user_hw && g_user_cb != g_user_over);
    /* the same name in another context: a code and a text of its own */
    g_user_other = SECoP_S_registerUserError("_Overheated", "coil B too hot", "uerr_cb");
    CHECK(g_user_other <= -1000 && g_user_other != g_user_over);
    CHECK(SECoP_S_registerUserError("_overheated", "x", "uerr") == SECoP_S_ERROR_NAME_ALREADY_USED);
    CHECK(SECoP_S_registerUserError("hardwareerror", "x", "uerr") == SECoP_S_ERROR_NAME_ALREADY_USED);
    CHECK(SECoP_S_registerUserError("over-heated", "x", "uerr") == SECoP_S_ERROR_INVALID_NAME);
    CHECK(SECoP_S_registerUserError("1st", "x", "uerr") == SECoP_S_ERROR_INVALID_NAME);
    CHECK(SECoP_S_registerUserError(NULL, "x", "uerr") == SECoP_S_ERROR_INVALID_NAME);
    return 0;
}

/* answers the next request of context "uerr" with the user error of "which":
   0 = _Overheated (with putCommandAnswer2), 1 = HardwareError (with putCommandAnswer),
   2 = _Overheated of context "uerr_cb" */
int ctest_user_error_answer(int which)
{
    unsigned long long id = 0;
    enum SECoP_S_action action = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int i;
    enum SECoP_S_error rc = SECoP_S_ERROR_NO_DATA;
    for (i = 0; i < 500 && rc == SECoP_S_ERROR_NO_DATA; ++i) {
        int name_size = (int)sizeof(name), value_size = (int)sizeof(value);
        rc = SECoP_S_getStoredCommand2(&id, &action, name, &name_size, value, &value_size, "uerr");
        if (rc == SECoP_S_ERROR_NO_DATA)
            sleep_ms(10);
    }
    OK(rc);
    if (which == 0)
        OK(SECoP_S_putCommandAnswer2(id, g_user_over, NULL, 0, NULL, 0, 0.0));
    else if (which == 1)
        OK(SECoP_S_putCommandAnswer(id, g_user_hw, NULL, NULL, 0.0));
    else
        OK(SECoP_S_putCommandAnswer2(id, g_user_other, NULL, 0, NULL, 0, 0.0));
    return 0;
}

/* registering again, unregistering and the lifetime of error strings */
int ctest_user_error_unregister(void)
{
    const char* kept = SECoP_S_getErrorString(g_user_over);
    enum SECoP_S_error again;
    /* again for the same context: the same code */
    CHECK(SECoP_S_registerUserError("_Overheated", "coil too hot", "uerr") == g_user_over);
    /* not by another context */
    CHECK(SECoP_S_unregisterUserError(g_user_over, "uerr_cb") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(strcmp(SECoP_S_getErrorString(g_user_over), "_Overheated: coil too hot") == 0);
    /* one unregister removes it */
    OK(SECoP_S_unregisterUserError(g_user_over, "uerr"));
    CHECK(SECoP_S_unregisterUserError(g_user_over, "uerr") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(strncmp(SECoP_S_getErrorString(g_user_over), "INVALID", 7) == 0);
    /* a string handed out before stays valid */
    CHECK(strcmp(kept, "_Overheated: coil too hot") == 0);
    /* the other context's code of the same name stays */
    CHECK(strcmp(SECoP_S_getErrorString(g_user_other), "_Overheated: coil B too hot") == 0);
    /* SHALL's codes are not registered */
    CHECK(SECoP_S_unregisterUserError(SECoP_S_ERROR_IS_BUSY, "uerr") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(SECoP_S_unregisterUserError(SECoP_S_SUCCESS, "uerr") == SECoP_S_ERROR_INVALID_VALUE);
    /* codes are never reused */
    again = SECoP_S_registerUserError("_Overheated", "coil too hot", "uerr");
    CHECK(again <= -1000 && again != g_user_over && again != g_user_hw && again != g_user_cb &&
          again != g_user_other);
    OK(SECoP_S_unregisterUserError(again, "uerr"));
    return 0;
}

/* the codes end with a full doneLibrary of their context */
int ctest_user_error_done(void)
{
    /* nodes only: the codes stay, and the context gets them back when initialized again */
    SECoP_S_doneLibrary(1, "uerr_cb");
    CHECK(strcmp(SECoP_S_getErrorString(g_user_cb), "_CbFault: from a callback") == 0);
    OK(SECoP_S_initLibrary(NULL, 0, 1, "uerr_cb"));
    CHECK(SECoP_S_registerUserError("_CbFault", "from a callback", "uerr_cb") == g_user_cb);
    SECoP_S_doneLibrary(0, "uerr_cb");
    CHECK(strncmp(SECoP_S_getErrorString(g_user_cb), "INVALID", 7) == 0);
    CHECK(strncmp(SECoP_S_getErrorString(g_user_other), "INVALID", 7) == 0);
    /* the other context's codes were not touched */
    CHECK(strcmp(SECoP_S_getErrorString(g_user_hw), "HardwareError: sensor disconnected") == 0);
    SECoP_S_doneLibrary(0, "uerr");
    CHECK(strncmp(SECoP_S_getErrorString(g_user_hw), "INVALID", 7) == 0);
    return 0;
}

/* --- filtered requests (must addition): the Rust side sends the requests */

static unsigned long long g_filter_id = 0;

static enum SECoP_S_error add_filter_param(const char* name, int writable, const char* context)
{
    enum SECoP_S_error rc = writable ? SECoP_S_addWritableParameter(name, NULL, NULL, context)
                                     : SECoP_S_addReadableParameter(name, NULL, context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyString("description", name, context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", context);
    return rc;
}

static enum SECoP_S_error add_filter_module(const char* name, const char* context)
{
    enum SECoP_S_error rc = SECoP_S_addModule(name, context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyString("description", "module", context);
    if (rc >= 0)
        rc = SECoP_S_addPropertyDouble("pollinterval", 0.0, context);
    return rc;
}

/* polling-mode nodes, not polled: "fn" (context "filt") with mod1:a, mod1:b (writable)
   and mod2:b; "fn2" (context "filt2") with mod1:a */
int ctest_filter_build(unsigned short port, unsigned short port2)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "filt"));
    OK(SECoP_S_createNode2("fn", "filtered requests", "127.0.0.1", port, "filt"));
    OK(add_filter_module("mod1", "filt"));
    OK(add_filter_param("a", 1, "filt"));
    OK(add_filter_param("b", 1, "filt"));
    OK(add_filter_module("mod2", "filt"));
    OK(add_filter_param("b", 0, "filt"));
    CHECK(complete("filt") >= 0);
    OK(SECoP_S_initLibrary(NULL, 0, 0, "filt2"));
    OK(SECoP_S_createNode2("fn2", "another context", "127.0.0.1", port2, "filt2"));
    OK(add_filter_module("mod1", "filt2"));
    OK(add_filter_param("a", 0, "filt2"));
    CHECK(complete("filt2") >= 0);
    return 0;
}

/* one call of SECoP_S_filterStoredCommand2 with large buffers; returns its result */
int ctest_filter_rc(const char* filter, int action, const char* context)
{
    enum SECoP_S_action got = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int name_size = (int)sizeof(name), value_size = (int)sizeof(value);
    return SECoP_S_filterStoredCommand2(filter, action, &g_filter_id, &got, name, &name_size, value, &value_size,
                                        context);
}

/* takes a request with the filter and checks it: name, action and value (a double, or
   NULL for none). variant 2: SECoP_S_filterStoredCommand2 with a name buffer of
   name_cap bytes; variant 1: SECoP_S_filterStoredCommand */
int ctest_filter_take(const char* filter, int action, const char* context, const char* expected,
                      int expected_action, const char* expected_value, int name_cap, int variant)
{
    enum SECoP_S_action got = SECoP_S_ACTION_NONE;
    char name[64], value[64];
    int name_size = name_cap, value_size = (int)sizeof(value);
    CSECoPbaseType* handle = NULL;
    double number = 0.0;
    enum SECoP_S_error rc;
    CHECK(name_cap > 0 && name_cap <= (int)sizeof(name));
    if (variant == 2)
        rc = SECoP_S_filterStoredCommand2(filter, action, &g_filter_id, &got, name, &name_size, value, &value_size,
                                          context);
    else
        rc = SECoP_S_filterStoredCommand(filter, action, &g_filter_id, &got, name, &name_size, &handle, context);
    if (name_cap > (int)strlen(expected)) {
        OK(rc);
        CHECK(strcmp(name, expected) == 0);
    } else {
        /* too small: handed out anyway, cut and NUL-terminated, with the needed size */
        CHECK(rc == SECoP_S_WARNING_BUFFER_TOO_SMALL);
        CHECK(strncmp(name, expected, (size_t)name_cap - 1) == 0 && name[name_cap - 1] == '\0');
    }
    CHECK(name_size == (int)strlen(expected));
    CHECK(g_filter_id != 0 && (int)got == expected_action);
    if (variant == 2) {
        CHECK(expected_value == NULL ? value[0] == '\0' : strtod(value, NULL) == strtod(expected_value, NULL));
    } else {
        CHECK((handle == NULL) == (expected_value == NULL));
        if (handle != NULL) {
            CHECK(SECoP_V_getDouble(handle, 0, 0, &number) && number == strtod(expected_value, NULL));
            SECoP_V_destroy(&handle);
        }
    }
    return 0;
}

/* answers the request taken last */
int ctest_filter_answer(const char* value)
{
    OK(SECoP_S_putCommandAnswer2(g_filter_id, SECoP_S_SUCCESS, value, -1, NULL, 0, 0.0));
    return 0;
}

/* --- stress */

int ctest_server_stress_build(unsigned short port)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "stress"));
    OK(SECoP_S_createNode2("snode", "stress", "127.0.0.1", port, "stress"));
    OK(SECoP_S_addModule("m", "stress"));
    OK(SECoP_S_addPropertyString("description", "stressed module", "stress"));
    OK(SECoP_S_addReadableParameter("value", NULL, "stress"));
    OK(SECoP_S_addPropertyString("description", "value", "stress"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "stress"));
    OK(SECoP_S_addWritableParameter("target", NULL, NULL, "stress"));
    OK(SECoP_S_addPropertyString("description", "target", "stress"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "stress"));
    CHECK(complete("stress") >= 0);
    return 0;
}

/* --- monitoring: a node that is created but not complete */

int ctest_monitor_building(void)
{
    OK(SECoP_S_initLibrary(NULL, 0, 0, "mon"));
    OK(SECoP_S_createNode2("mnode", "being built", "127.0.0.1", 0, "mon"));
    OK(SECoP_S_addModule("m", "mon"));
    return 0;
}

/* --- the status window flags open the status TUI; the calls always succeed */

int ctest_status_window(void)
{
    OK(SECoP_S_initLibrary(NULL, 1, 1, "gui"));
    SECoP_S_showStatusWindow(1);
    SECoP_S_showStatusWindow(0);
    SECoP_S_doneLibrary(1, "gui");
    return 0;
}

/* --- application log messages (must addition): node "my_node", module mod1 in callback
   mode, whose getter logs for its own module while the node answers the read */

static volatile int g_log_getter_rc = 1;

static void get_logging(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                        CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)ppSigma;
    (void)timestamp;
    g_log_getter_rc = SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_INFO, "read requested");
    SECoP_V_modifyDouble(*ppData, 0, 0, 1.0);
    *piError = SECoP_S_SUCCESS;
}

int ctest_log_build(unsigned short port)
{
    g_log_getter_rc = 1;
    OK(SECoP_S_initLibrary(NULL, 0, 1, "log"));
    OK(SECoP_S_createNode2("my_node", "logs", "127.0.0.1", port, "log"));
    OK(SECoP_S_addModule("mod1", "log"));
    OK(SECoP_S_addPropertyString("description", "logging module", "log"));
    OK(SECoP_S_addPropertyDouble("pollinterval", 0.0, "log"));
    OK(SECoP_S_addReadableParameter("value", get_logging, "log"));
    OK(SECoP_S_addPropertyString("description", "value", "log"));
    OK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\"}", "log"));
    /* not complete yet */
    CHECK(SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_INFO, "x") == SECoP_S_ERROR_INVALID_NODE);
    CHECK(complete("log") >= 0);
    CHECK(SECoP_S_logMessage("my_node:nope", SECoP_S_LOG_INFO, "x") == SECoP_S_ERROR_INVALID_MODULE);
    CHECK(SECoP_S_logMessage("other:mod1", SECoP_S_LOG_INFO, "x") == SECoP_S_ERROR_INVALID_NODE);
    CHECK(SECoP_S_logMessage("my_node:mod1", (enum SECoP_S_logLevel)0, "x") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(SECoP_S_logMessage("my_node:mod1", (enum SECoP_S_logLevel)5, "x") == SECoP_S_ERROR_INVALID_VALUE);
    CHECK(SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_INFO, NULL) == SECoP_S_ERROR_INVALID_VALUE);
    return 0;
}

/* SECoP_S_logMessage for my_node:mod1 */
int ctest_log_message(int level, const char* text)
{
    return SECoP_S_logMessage("my_node:mod1", (enum SECoP_S_logLevel)level, text);
}

/* what SECoP_S_logMessage returned in the getter, 1 before it ran */
int ctest_log_getter_rc(void)
{
    return g_log_getter_rc;
}
