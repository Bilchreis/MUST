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
        return 0;

    default:
        return __LINE__;
    }
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
