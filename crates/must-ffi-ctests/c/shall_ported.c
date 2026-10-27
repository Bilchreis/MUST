/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* The scenarios of SHALL's own tests (gtest-serverlib/tst_contextid.cpp,
   tst_protocol.cpp and their node test_node.cpp), ported to C. The node is SHALL's
   "hpd" hotplate, built exactly as SHALL's tests build it. The checks that looked into
   SHALL's Qt internals (node list, status window tabs) are replaced by what a client
   sees; the Rust side (tests/shall_ported.rs) does the networking. Each test returns 0
   on success or the line of the first failed check. */

#if !defined(_WIN32)
#define _POSIX_C_SOURCE 199309L
#endif

#include "SECoP.h"

#include <stdio.h>
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

#define CHECK(cond)                                                                    \
    do {                                                                               \
        if (!(cond)) {                                                                 \
            fprintf(stderr, "shall_ported.c:%d: check failed: %s\n", __LINE__, #cond); \
            return __LINE__;                                                           \
        }                                                                              \
    } while (0)

/* --- test_node.cpp */

static void Local_GetTemperature(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                                 CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)piError;
    (void)timestamp;
    *ppSigma = SECoP_V_create("{\"type\":\"double\"}");
    SECoP_V_modifyDouble(*ppData, 0, 0, 0.5); /* SHALL used a random number */
    SECoP_V_modifyDouble(*ppSigma, 0, 0, 0.01); /* fixed error value */
}

static void Local_GetTarget(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                            CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)piError;
    (void)ppSigma;
    (void)timestamp;
    SECoP_V_modifyDouble(*ppData, 0, 0, 20);
}

static void Local_SetTarget(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                            CSECoPbaseType** ppSigma, double* timestamp)
{
    double dblTarget = 0.0;
    (void)name;
    (void)ppSigma;
    (void)timestamp;
    if (SECoP_V_getDouble(*ppData, 0, 0, &dblTarget)) {
        if (dblTarget < -273.15)
            dblTarget = -273.15;
        else if (dblTarget > 1000.0)
            dblTarget = 1000.0;
        SECoP_V_modifyDouble(*ppData, 0, 0, dblTarget);
    } else
        *piError = SECoP_S_ERROR_INVALID_VALUE;
}

static void Local_GetStatus(const char* name, enum SECoP_S_error* piError, CSECoPbaseType** ppData,
                            CSECoPbaseType** ppSigma, double* timestamp)
{
    (void)name;
    (void)piError;
    (void)ppSigma;
    (void)timestamp;
    SECoP_V_modifyInteger(*ppData, 1, 0, 100);
    SECoP_V_modifyString(*ppData, 2, "IDLE Counting flowers on the wall", -1);
}

static void funcCall(const char* name, const CSECoPbaseType* pArgument, enum SECoP_S_error* piError,
                     CSECoPbaseType** ppReturn, double* timestamp)
{
    (void)name;
    (void)pArgument;
    (void)piError;
    (void)ppReturn;
    (void)timestamp;
}

/* Node_no_wait of test_node.cpp; every call must succeed (or warn) */
static int Node_no_wait(const char* context_id, const char* Node_id, unsigned short port, int enablefunctionpointers)
{
    CHECK(SECoP_S_initLibrary(NULL, 1, enablefunctionpointers, context_id) == SECoP_S_SUCCESS);
    SECoP_S_setManyThreads(0);
    CHECK(SECoP_S_createNode(Node_id, "TestNode", port, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addModule("hpd", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyString("description", "Hotplate drivable", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyJSON("interface_classes", "[\"Drivable\",\"Writable\",\"Readable\"]", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyDouble("pollinterval", 10.0, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addReadableParameter("value", &Local_GetTemperature, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\",\"unit\":\"K\"}", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyDouble("pollinterval", 1.0, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyString("description", "actual temperature", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addReadableParameter("status", &Local_GetStatus, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyJSON("datainfo",
                                  "{\"type\":\"tuple\",\"members\":[{\"type\":\"enum\",\"members\":{\"IDLE\":100,\"WARN\":200,"
                                  "\"BUSY\":300,\"BUSY_Stabilizing\":380,\"ERROR\":400,\"DISABLED\":0}},{\"type\":\"string\"}]}",
                                  context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyString("description", "machine status", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyDouble("pollinterval", 1.0, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addWritableParameter("target", &Local_GetTarget, &Local_SetTarget, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyJSON("datainfo", "{\"type\":\"double\",\"unit\":\"K\"}", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyString("description", "target temperature", context_id) == SECoP_S_SUCCESS);
    /* no datainfo: SHALL's default {"type":"command"} */
    CHECK(SECoP_S_addCommand("stop", &funcCall, context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_addPropertyString("description", "stops and settings are not stored no resume", context_id) == SECoP_S_SUCCESS);
    CHECK(SECoP_S_nodeComplete(context_id) >= 0);
    SECoP_S_showStatusWindow(1);
    return 0;
}

int ctest_shall_node(const char* context_id, const char* node_id, unsigned short port, int enablefunctionpointers)
{
    return Node_no_wait(context_id, node_id, port, enablefunctionpointers);
}

int ctest_shall_delete_node(const char* node_id)
{
    CHECK(SECoP_S_deleteNode(node_id) == SECoP_S_SUCCESS);
    return 0;
}

int ctest_shall_done(int bNodeOnly, const char* context_id)
{
    SECoP_S_doneLibrary(bNodeOnly, context_id);
    return 0;
}

/* tst_contextid.cpp GetCommad: after the Rust side sent "change hpd:target 500" to the
   nodes of both contexts, each context gets only the request of its own node */
int ctest_shall_get_command(const char* context_id, const char* expected_name)
{
    unsigned long long pllId = 10;
    enum SECoP_S_action Action = SECoP_S_ACTION_NONE;
    char szParameter[40];
    int piParameterSize = 40;
    int i;
    double value = 0.0;
    CSECoPbaseType* ppValue = SECoP_V_fromJSON("{\"type\":\"double\",\"unit\":\"K\"}", NULL);
    CSECoPbaseType* passed = ppValue;

    /* SHALL's test spun without limit; polled reads of the node are skipped */
    for (i = 0; i < 1000 && Action != SECoP_S_ACTION_CHANGE; ++i) {
        pllId = 10;
        Action = SECoP_S_ACTION_NONE;
        piParameterSize = 40;
        if (SECoP_S_getStoredCommand(&pllId, &Action, szParameter, &piParameterSize, &ppValue, context_id) == SECoP_S_ERROR_NO_DATA)
            sleep_ms(10);
        else if (Action != SECoP_S_ACTION_CHANGE)
            SECoP_V_destroy(&ppValue); /* NULL for a read */
    }
    CHECK(Action == SECoP_S_ACTION_CHANGE);
    CHECK(strcmp(szParameter, expected_name) == 0);
    CHECK(ppValue != NULL && SECoP_V_getDouble(ppValue, 0, 0, &value) && value == 500.0);
    CHECK(SECoP_S_putCommandAnswer(pllId, SECoP_S_SUCCESS, ppValue, NULL, 0) == SECoP_S_SUCCESS);
    SECoP_V_destroy(&ppValue);
    SECoP_V_destroy(&passed); /* SHALL's test leaked the handle it passed in */
    return 0;
}
