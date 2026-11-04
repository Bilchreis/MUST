/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* Compiles SHALL's documentation example against MUST's headers, as C99 and C++11.
   Only checked for compiling here; the C conformance tests run it. */

#include "SECoP.h"

#include <string.h>

/* MUST addition: a check function that accepts everything */
static void header_check_accept(const char* name, const CSECoPbaseType* pValue, enum SECoP_S_error* piError)
{
    (void)name;
    (void)pValue;
    *piError = SECoP_S_SUCCESS;
}

int run_header_example(int allowed_to_loop)
{
    /* create a very limited SEC node without Qt nor function pointers (from SECoP.h) */
    SECoP_S_initLibrary(0, 1, 0, "example");
    SECoP_S_createNode("my_node", "best SEC node ever", 2055, "example");
    SECoP_S_addModule("mod1", "example");
    SECoP_S_addPropertyString("description", "the first module", "example");
    SECoP_S_addReadableParameter("value", 0, "example");
    SECoP_S_addPropertyString("description", "actual value", "example");
    SECoP_S_addPropertyString("datainfo", "{\"type\":\"double\", \"unit\":\"mm\"}", "example");
    SECoP_S_nodeComplete("example");
    while (allowed_to_loop--)
    {
        unsigned long long id;
        enum SECoP_S_action action;
        enum SECoP_S_error error;
        char parameter[50], value[1000];
        int parameter_size = 50, value_size = 1000;
        if (SECoP_S_getStoredCommand2(&id, &action, parameter, &parameter_size, value, &value_size, "example") >= 0)
        {
            error = SECoP_S_SUCCESS;
            SECoP_S_putCommandAnswer2(id, error, value, (int)strlen(value), 0, 0, 0.0);
        }
    }
    SECoP_S_doneLibrary(0, "example");

    /* MUST additions */
    {
        unsigned long long id;
        enum SECoP_S_action action;
        char accessible[50], value[1000];
        int accessible_size = 50, value_size = 1000;
        CSECoPbaseType* p = 0;
        enum SECoP_S_error code = SECoP_S_registerUserError("_HeaderCheck", "from the header check", "example");
        SECoP_S_filterStoredCommand("my_node:mod1", 0, &id, &action, accessible, &accessible_size, &p, "example");
        SECoP_S_filterStoredCommand2("my_node:mod1:value", SECoP_S_ACTION_READ, &id, &action, accessible,
                                     &accessible_size, value, &value_size, "example");
        SECoP_S_unregisterUserError(code, "example");
        SECoP_S_filterStoredCommand2("my_node:mod1", SECoP_S_ACTION_CHECK, &id, &action, accessible,
                                     &accessible_size, value, &value_size, "example");
        {
            SECoP_S_checkFunction check = header_check_accept;
            SECoP_S_setCheckFunction(check, "example");
            SECoP_S_setCheckFunction(0, "example");
        }
        SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_INFO, "from the header check");
        SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_ERROR, "e");
        SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_WARNING, "w");
        SECoP_S_logMessage("my_node:mod1", SECoP_S_LOG_DEBUG, "d");
    }

    /* MUST additions for matrices */
    {
        const unsigned int len[2] = {1, 1};
        const float data[1] = {1.0f};
        SECoP_S_updateParameterMatrix("my_node:mod1:image", len, 2, data, (int)sizeof data, 0.0);
        SECoP_S_putCommandAnswerMatrix(1, SECoP_S_SUCCESS, len, 2, data, (int)sizeof data, 0.0);
    }

    {
        CSECoPbaseType* p = SECoP_V_createSimpleDouble(1.5);
        double d = 0.0;
        SECoP_V_getDouble(p, 0, 0, &d);
        SECoP_V_destroy(&p);
    }
    return 0;
}
