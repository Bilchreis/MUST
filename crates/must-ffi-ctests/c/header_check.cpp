/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* The same example compiled as C++, to check the headers for C++ callers (including the
   MUST additions such as SECoP_S_logMessage with its enum SECoP_S_logLevel). The function
   has C linkage, so that tests/header.rs can check that it links. */
#define run_header_example run_header_example_cpp
extern "C" int run_header_example(int allowed_to_loop);
#include "header_check.c"
