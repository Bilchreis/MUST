/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* Compiles SHALL's documentation example against must's headers, as C99 and C++11.
   Only checked for compiling here; the C conformance tests run it. */

#include "SECoP.h"

#include <string.h>

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

    {
        CSECoPbaseType* p = SECoP_V_createSimpleDouble(1.5);
        double d = 0.0;
        SECoP_V_getDouble(p, 0, 0, &d);
        SECoP_V_destroy(&p);
    }
    return 0;
}
