/* SPDX-License-Identifier: LGPL-3.0-or-later */
/* C conformance tests of the SECoP_V_* functions. Each test returns 0 on success or the
   line of the first failed check. */

#include "SECoP.h"

#include <limits.h>
#include <math.h>
#include <stdio.h>
#include <string.h>

#define CHECK(cond)                                                       \
    do {                                                                  \
        if (!(cond)) {                                                    \
            fprintf(stderr, "variant.c:%d: check failed: %s\n", __LINE__, #cond); \
            return __LINE__;                                              \
        }                                                                 \
    } while (0)

static int json_is(CSECoPbaseType* p, const char* expected)
{
    char buf[512];
    int n = SECoP_V_toJSON(p, buf, (int)sizeof(buf));
    if (n <= 0 || strcmp(buf, expected) != 0) {
        fprintf(stderr, "  toJSON gave '%s', expected '%s'\n", n > 0 ? buf : "(error)", expected);
        return 0;
    }
    return 1;
}

static unsigned int type_at(CSECoPbaseType* p, unsigned int pos)
{
    enum SECoP_V_type t = SECoP_VT_NONE;
    unsigned int count = 0;
    const char* name = NULL;
    if (!SECoP_V_getInfo(p, pos, &t, &count, &name))
        return 999;
    return (unsigned int)t;
}

int ctest_variant_create(void)
{
    CSECoPbaseType* p = SECoP_V_createSimpleDouble(1.5);
    CSECoPbaseType* q = NULL;
    char buf[256];

    /* create and destroy */
    CHECK(p != NULL);
    CHECK(json_is(p, "1.5"));
    SECoP_V_destroy(&p);
    CHECK(p == NULL);

    /* invalid description */
    CHECK(SECoP_V_create("{\"type\":\"nope\"}") == NULL);
    CHECK(SECoP_V_create("not json") == NULL);
    CHECK(SECoP_V_create(NULL) == NULL);

    /* simple values */
    p = SECoP_V_createSimpleBool(1);
    CHECK(json_is(p, "true") && type_at(p, 0) == SECoP_VT_BOOL);
    SECoP_V_destroy(&p);
    p = SECoP_V_createSimpleInteger(-7);
    CHECK(json_is(p, "-7") && type_at(p, 0) == SECoP_VT_INTEGER);
    SECoP_V_destroy(&p);
    p = SECoP_V_createSimpleScaled(0.1, 1255);
    CHECK(json_is(p, "1255") && type_at(p, 0) == SECoP_VT_SCALED);
    SECoP_V_destroy(&p);
    CHECK(SECoP_V_createSimpleScaled(0.0, 1) == NULL);
    p = SECoP_V_createSimpleString("hello");
    CHECK(json_is(p, "\"hello\"") && type_at(p, 0) == SECoP_VT_STRING);
    SECoP_V_destroy(&p);
    p = SECoP_V_createSimpleString(NULL);
    CHECK(json_is(p, "\"\""));
    SECoP_V_destroy(&p);

    /* status */
    p = SECoP_V_createSECoPStatus();
    CHECK(p != NULL && type_at(p, 0) == SECoP_VT_TUPLE && type_at(p, 1) == SECoP_VT_ENUM);
    CHECK(json_is(p, "[100,\"\"]"));
    CHECK(SECoP_V_getType(p, buf, (int)sizeof(buf)) > 0);
    CHECK(strcmp(buf, "{\"type\":\"tuple\",\"members\":[{\"type\":\"enum\",\"members\":{\"IDLE\":100}},{\"type\":\"string\"}]}") == 0);

    /* copy */
    CHECK(SECoP_V_copy(&q, p) == 1 && q != NULL && q != p);
    CHECK(SECoP_V_compare(p, q) == SECoP_VC_EQUAL);
    SECoP_V_destroy(&q);
    CHECK(SECoP_V_copy(NULL, p) == 0);
    SECoP_V_destroy(&p);

    /* from a description, with default values */
    p = SECoP_V_create("{\"type\":\"array\",\"members\":{\"type\":\"int\"},\"minlen\":2,\"maxlen\":5}");
    CHECK(p != NULL && type_at(p, 0) == SECoP_VT_ARRAY_INTEGER && json_is(p, "[0,0]"));
    SECoP_V_destroy(&p);
    p = SECoP_V_create("{\"type\":\"Double\",\"min\":1,\"max\":5}");
    CHECK(p != NULL && json_is(p, "1.0"));
    SECoP_V_destroy(&p);
    return 0;
}

int ctest_variant_json(void)
{
    CSECoPbaseType* hint = SECoP_V_create("{\"type\":\"scaled\",\"scale\":0.1,\"min\":0,\"max\":2500}");
    CSECoPbaseType* p;
    char small[10], big[64];
    double d = 0.0;
    int needed;

    /* JSON with datainfo hint */
    CHECK(hint != NULL);
    p = SECoP_V_fromJSON("1255", hint);
    CHECK(p != NULL && type_at(p, 0) == SECoP_VT_SCALED);
    CHECK(SECoP_V_getDouble(p, 0, 0, &d) == 1 && fabs(d - 125.5) < 1e-9);
    SECoP_V_destroy(&p);
    CHECK(SECoP_V_fromJSON("2501", hint) == NULL); /* outside the hint's limits */
    CHECK(SECoP_V_fromJSON("\"x\"", hint) == NULL);
    SECoP_V_destroy(&hint);

    /* buffer too small: 20 bytes of JSON, 10-byte buffer */
    p = SECoP_V_fromJSON("\"abcdefghijklmnopq\"", NULL); /* 19 chars of JSON + NUL */
    CHECK(p != NULL);
    memset(small, 'X', sizeof(small));
    needed = SECoP_V_toJSON(p, small, (int)sizeof(small));
    CHECK(needed == 20);
    CHECK(small[9] == '\0' && strncmp(small, "\"abcdefgh", 9) == 0);
    CHECK(SECoP_V_toJSON(p, NULL, 0) == 20);
    CHECK(SECoP_V_toJSON(p, big, needed) == 20 && strcmp(big, "\"abcdefghijklmnopq\"") == 0);
    SECoP_V_destroy(&p);

    /* inferred types */
    p = SECoP_V_fromJSON("[1.0, 2.5]", NULL);
    CHECK(type_at(p, 0) == SECoP_VT_ARRAY_DOUBLE && json_is(p, "[1.0,2.5]"));
    SECoP_V_destroy(&p);
    p = SECoP_V_fromJSON("{\"a\": 1, \"b\": [true, false]}", NULL);
    CHECK(type_at(p, 0) == SECoP_VT_STRUCT && type_at(p, 1) == SECoP_VT_INTEGER && type_at(p, 2) == SECoP_VT_ARRAY_BOOL);
    CHECK(json_is(p, "{\"a\":1,\"b\":[true,false]}"));
    SECoP_V_destroy(&p);
    p = SECoP_V_fromJSON("null", NULL);
    CHECK(p != NULL && type_at(p, 0) == SECoP_VT_NONE && json_is(p, "null"));
    CHECK(SECoP_V_getType(p, big, (int)sizeof(big)) < 0);
    SECoP_V_destroy(&p);
    CHECK(SECoP_V_fromJSON("[]", NULL) == NULL);
    CHECK(SECoP_V_fromJSON("{oops", NULL) == NULL);

    /* blobs travel as base64 */
    hint = SECoP_V_create("{\"type\":\"blob\",\"maxbytes\":8}");
    p = SECoP_V_fromJSON("\"U0VDb1A=\"", hint);
    CHECK(p != NULL && type_at(p, 0) == SECoP_VT_BLOB);
    CHECK(SECoP_V_getString(p, 0, sizeof(big), big) == 6 && strcmp(big, "SECoP") == 0);
    CHECK(json_is(p, "\"U0VDb1A=\""));
    SECoP_V_destroy(&p);
    SECoP_V_destroy(&hint);
    return 0;
}

int ctest_variant_null(void)
{
    CSECoPbaseType* p = NULL;
    CSECoPbaseType* stale = SECoP_V_createSimpleDouble(1.0);
    CSECoPbaseType* copy = stale;
    char buf[100];
    double d = 0.0;
    long long ll = 0;
    enum SECoP_V_type t;
    unsigned int n;

    CHECK(SECoP_V_toJSON(NULL, buf, 100) < 0);
    CHECK(SECoP_V_getType(NULL, buf, 100) < 0);
    CHECK(SECoP_V_getDouble(NULL, 0, 0, &d) == 0);
    CHECK(SECoP_V_getInteger(NULL, 0, 0, &ll) == 0);
    CHECK(SECoP_V_getInfo(NULL, 0, &t, &n, NULL) == 0);
    CHECK(SECoP_V_getString(NULL, 0, sizeof(buf), buf) == 0);
    CHECK(SECoP_V_modifyDouble(NULL, 0, 0, 1.0) == 0);
    CHECK(SECoP_V_compare(NULL, NULL) == SECoP_VC_DIFF_TYPE);
    SECoP_V_destroy(NULL);
    SECoP_V_destroy(&p);

    /* a destroyed handle is rejected, not used */
    SECoP_V_destroy(&stale);
    CHECK(SECoP_V_toJSON(copy, buf, 100) < 0);
    CHECK(SECoP_V_getDouble(copy, 0, 0, &d) == 0);
    SECoP_V_destroy(&copy); /* destroying twice is harmless */
    CHECK(copy == NULL);
    return 0;
}

int ctest_variant_positions(void)
{
    CSECoPbaseType* p = SECoP_V_fromJSON("[1.0, 2.5]", NULL);
    enum SECoP_V_type t = SECoP_VT_NONE;
    unsigned int count = 0, len;
    const char* name = "unchanged";
    char nbuf[8];
    double d = -1.0;

    /* an array of doubles is one position */
    CHECK(SECoP_V_getInfo(p, 0, &t, &count, &name) == 1);
    CHECK(t == SECoP_VT_ARRAY_DOUBLE && count == 2 && name == NULL);
    CHECK(SECoP_V_getDouble(p, 0, 1, &d) == 1 && d == 2.5);
    CHECK(SECoP_V_getInfo(p, 1, &t, &count, &name) == 0);

    /* out of range: 0 and output unchanged */
    d = -1.0;
    CHECK(SECoP_V_getDouble(p, 0, 2, &d) == 0 && d == -1.0);
    CHECK(SECoP_V_getDouble(p, 1, 0, &d) == 0 && d == -1.0);
    SECoP_V_destroy(&p);

    /* nested positions */
    p = SECoP_V_fromJSON("{\"a\": 1.0, \"b\": \"x\"}", NULL);
    CHECK(SECoP_V_getInfo(p, 0, &t, &count, &name) == 1 && t == SECoP_VT_STRUCT && count == 2 && name == NULL);
    CHECK(SECoP_V_getInfo(p, 1, &t, &count, &name) == 1 && t == SECoP_VT_DOUBLE && count == 1 && strcmp(name, "a") == 0);
    CHECK(SECoP_V_getInfo(p, 2, &t, &count, &name) == 1 && t == SECoP_VT_STRING && count == 1 && strcmp(name, "b") == 0);
    CHECK(SECoP_V_getInfo(p, 3, &t, &count, &name) == 0);
    CHECK(strcmp(SECoP_V_getStructName(p, 2), "b") == 0 && SECoP_V_getStructName(p, 0) == NULL);

    /* getInfo2 copies the name and reports its length */
    len = sizeof(nbuf);
    CHECK(SECoP_V_getInfo2(p, 1, &t, &count, nbuf, &len) == 1 && len == 1 && strcmp(nbuf, "a") == 0);
    len = sizeof(nbuf);
    CHECK(SECoP_V_getInfo2(p, 0, &t, &count, nbuf, &len) == 1 && len == 0);
    SECoP_V_destroy(&p);

    /* type codes */
    p = SECoP_V_create("{\"type\":\"struct\",\"members\":{\"s\":{\"type\":\"scaled\",\"scale\":0.5,\"min\":0,\"max\":10}}}");
    CHECK(type_at(p, 0) == SECoP_VT_STRUCT && type_at(p, 1) == SECoP_VT_SCALED);
    SECoP_V_destroy(&p);

    /* deeper nesting: items of a tuple inside a struct have no name */
    p = SECoP_V_fromJSON("{\"p\": [1, \"z\"], \"q\": true}", NULL);
    CHECK(type_at(p, 1) == SECoP_VT_TUPLE && type_at(p, 2) == SECoP_VT_INTEGER && type_at(p, 3) == SECoP_VT_STRING);
    CHECK(SECoP_V_getStructName(p, 2) == NULL && strcmp(SECoP_V_getStructName(p, 4), "q") == 0);
    SECoP_V_destroy(&p);
    return 0;
}

int ctest_variant_modify(void)
{
    CSECoPbaseType* p = SECoP_V_create("{\"type\":\"double\",\"min\":0,\"max\":10}");
    double dmin, dmax, d;
    long long lmin, lmax, ll;
    unsigned int len, umin, umax;
    char buf[16];

    /* double limits */
    CHECK(SECoP_V_getMinMaxDouble(p, 0, &dmin, &dmax) == 1 && dmin == 0.0 && dmax == 10.0);
    CHECK(SECoP_V_modifyDouble(p, 0, 0, 5.0) == 1 && SECoP_V_getDouble(p, 0, 0, &d) == 1 && d == 5.0);
    CHECK(SECoP_V_modifyDouble(p, 0, 0, 11.0) == 0);
    CHECK(SECoP_V_modifyMinMaxDouble(p, 0, 0.0, 20.0) == 1 && SECoP_V_modifyDouble(p, 0, 0, 11.0) == 1);
    CHECK(SECoP_V_modifyMinMaxDouble(p, 0, 5.0, 1.0) == 0);
    CHECK(SECoP_V_modifyMinMaxDouble(p, 0, NAN, NAN) == 1 && SECoP_V_modifyDouble(p, 0, 0, -1e9) == 1);
    CHECK(SECoP_V_modifyInteger(p, 0, 0, 1) == 0); /* not an integer type */
    SECoP_V_destroy(&p);

    /* integers, scaled */
    p = SECoP_V_create("{\"type\":\"scaled\",\"scale\":0.1,\"min\":0,\"max\":100}");
    CHECK(SECoP_V_getMinMaxInteger(p, 0, &lmin, &lmax) == 1 && lmin == 0 && lmax == 100);
    CHECK(SECoP_V_modifyDouble(p, 0, 0, 2.5) == 1 && SECoP_V_getInteger(p, 0, 0, &ll) == 1 && ll == 25);
    CHECK(SECoP_V_modifyInteger(p, 0, 0, 101) == 0);
    CHECK(SECoP_V_modifyMinMaxInteger(p, 0, 0, 1000) == 1 && SECoP_V_modifyInteger(p, 0, 0, 101) == 1);
    CHECK(SECoP_V_getScale(p, 0, &d) == 1 && d == 0.1);
    CHECK(SECoP_V_modifyScale(p, 0, 0.5) == 1 && SECoP_V_getDouble(p, 0, 0, &d) == 1 && d == 50.5);
    CHECK(SECoP_V_modifyScale(p, 0, -1.0) == 0);
    SECoP_V_destroy(&p);

    p = SECoP_V_createSimpleBool(0);
    CHECK(SECoP_V_modifyInteger(p, 0, 0, 1) == 1 && SECoP_V_modifyInteger(p, 0, 0, 2) == 0);
    SECoP_V_destroy(&p);

    /* array length */
    p = SECoP_V_create("{\"type\":\"array\",\"members\":{\"type\":\"double\"},\"maxlen\":4}");
    CHECK(SECoP_V_getArrayLength(p, 0, &len, &umin, &umax) == 1 && len == 0 && umin == 0 && umax == 4);
    CHECK(SECoP_V_modifyArrayLength(p, 0, 3, 1, 0) == 1 && json_is(p, "[0.0,0.0,0.0]"));
    CHECK(SECoP_V_modifyDouble(p, 0, 2, 7.5) == 1 && json_is(p, "[0.0,0.0,7.5]"));
    CHECK(SECoP_V_modifyArrayLength(p, 0, 5, 1, 0) == 0);           /* beyond maxlen */
    CHECK(SECoP_V_modifyArrayLength(p, 0, 5, 2, 8) == 1);           /* new limits first */
    CHECK(SECoP_V_getArrayLength(p, 0, &len, &umin, &umax) == 1 && len == 5 && umin == 2 && umax == 8);
    CHECK(SECoP_V_modifyArrayLength(p, 0, 1, 1, 0) == 0);           /* below minlen */
    SECoP_V_destroy(&p);

    /* strings */
    p = SECoP_V_create("{\"type\":\"string\",\"maxchars\":10}");
    CHECK(SECoP_V_modifyString(p, 0, "abcdefgh", 3) == 1 && json_is(p, "\"abc\""));
    CHECK(SECoP_V_modifyString(p, 0, "hello", -1) == 1 && json_is(p, "\"hello\""));
    CHECK(SECoP_V_modifyString(p, 0, "this is too long", -1) == 0);
    CHECK(SECoP_V_getString(p, 0, 0, NULL) == 6);
    memset(buf, 'X', sizeof(buf));
    CHECK(SECoP_V_getString(p, 0, 3, buf) == 6 && strcmp(buf, "he") == 0);
    CHECK(SECoP_V_getString(p, 0, 4, NULL) == 0);
    CHECK(SECoP_V_getArrayLength(p, 0, &len, &umin, &umax) == 1 && len == 5 && umax == 10);
    SECoP_V_destroy(&p);
    return 0;
}

int ctest_variant_enum_struct(void)
{
    CSECoPbaseType* p = SECoP_V_create("{\"type\":\"enum\",\"members\":{\"IDLE\":100,\"BUSY\":300}}");
    long long v = 0;
    const char* name;

    CHECK(SECoP_V_getEnumStructCount(p, 0) == 2);
    name = SECoP_V_getEnumeration(p, 0, 1, &v);
    CHECK(name != NULL && strcmp(name, "BUSY") == 0 && v == 300);
    CHECK(SECoP_V_getEnumeration(p, 0, 2, &v) == NULL);
    CHECK(SECoP_V_modifyInteger(p, 0, 0, 300) == 1 && SECoP_V_modifyInteger(p, 0, 0, 200) == 0);
    CHECK(SECoP_V_putEnumeration(p, 0, 400, "ERROR") == 1 && SECoP_V_getEnumStructCount(p, 0) == 3);
    CHECK(SECoP_V_modifyInteger(p, 0, 0, 400) == 1);
    CHECK(SECoP_V_putEnumeration(p, 0, 100, "OTHER") == 0); /* value used by IDLE */
    CHECK(SECoP_V_putEnumeration(p, 0, 150, "IDLE") == 1);  /* changes IDLE */
    CHECK(SECoP_V_putEnumeration(p, 0, 1, "") == 0);
    CHECK(SECoP_V_putEnumeration(p, 0, 1, NULL) == 0);
    SECoP_V_destroy(&p);

    p = SECoP_V_fromJSON("{\"x\": 1, \"y\": 2, \"z\": 3}", NULL);
    CHECK(SECoP_V_getEnumStructCount(p, 0) == 3 && SECoP_V_getEnumStructCount(p, 1) == 0);
    CHECK(strcmp(SECoP_V_getStructName(p, 3), "z") == 0);
    CHECK(SECoP_V_getEnumeration(p, 0, 0, &v) == NULL);
    SECoP_V_destroy(&p);
    return 0;
}

int ctest_variant_additional(void)
{
    CSECoPbaseType* p = SECoP_V_create(
        "{\"type\":\"double\",\"unit\":\"K\",\"fmtstr\":\"%.3f\",\"_scale\":2,\"_list\":[1,2]}");
    char buf[32];

    CHECK(SECoP_V_getAdditionalCount(p, 0) == 4);
    CHECK(SECoP_V_getAdditionalIndex(p, 0, "UNIT") == 0);
    CHECK(SECoP_V_getAdditionalIndex(p, 0, "nope") == UINT_MAX);
    CHECK(SECoP_V_getAdditionalKey(p, 0, 1, sizeof(buf), buf) == 7 && strcmp(buf, "fmtstr") == 0);
    CHECK(SECoP_V_getAdditionalString(p, 0, 0, sizeof(buf), buf) == 2 && strcmp(buf, "K") == 0);
    CHECK(SECoP_V_getAdditionalString(p, 0, 3, sizeof(buf), buf) == 6 && strcmp(buf, "[1,2]") == 0);
    CHECK(SECoP_V_getAdditionalDouble(p, 0, 2) == 2.0 && SECoP_V_getAdditionalInteger(p, 0, 2) == 2);
    CHECK(isnan(SECoP_V_getAdditionalDouble(p, 0, 0)));
    CHECK(SECoP_V_getAdditionalKey(p, 0, 4, sizeof(buf), buf) == 0);
    CHECK(SECoP_V_getAdditionalCount(p, 1) == 0);
    /* the additional items are part of the type */
    CHECK(SECoP_V_getType(p, buf, sizeof(buf)) > (int)sizeof(buf));
    SECoP_V_destroy(&p);
    return 0;
}

int ctest_variant_compare_print(void)
{
    CSECoPbaseType* a = SECoP_V_createSimpleDouble(1.5);
    CSECoPbaseType* b = SECoP_V_createSimpleDouble(1.5);
    CSECoPbaseType* s = SECoP_V_createSimpleString("x");
    CSECoPbaseType* limited = SECoP_V_create("{\"type\":\"double\",\"min\":0,\"max\":10}");
    FILE* f;
    char text[128];
    size_t n;

    CHECK(SECoP_V_compare(a, b) == SECoP_VC_EQUAL);
    CHECK(SECoP_V_compare(a, s) == SECoP_VC_DIFF_TYPE);
    CHECK(SECoP_V_modifyDouble(limited, 0, 0, 1.5) == 1);
    CHECK(SECoP_V_compare(a, limited) == SECoP_VC_SIMILAR_VALUE);
    CHECK(SECoP_V_modifyDouble(b, 0, 0, 2.0) == 1);
    CHECK(SECoP_V_compare(a, b) == SECoP_VC_DIFF_VALUE);

    /* printing a struct */
    SECoP_V_destroy(&s);
    s = SECoP_V_fromJSON("{\"x\": 1.5, \"y\": \"text\"}", NULL);
    f = tmpfile();
    CHECK(f != NULL);
    SECoP_V_printFile(f, s, 1);
    rewind(f);
    n = fread(text, 1, sizeof(text) - 1, f);
    text[n] = '\0';
    fclose(f);
    CHECK(strstr(text, "{\"x\":1.5,\"y\":\"text\"}") != NULL);
    CHECK(strstr(text, "\"type\":\"struct\"") != NULL);

    SECoP_V_destroy(&a);
    SECoP_V_destroy(&b);
    SECoP_V_destroy(&s);
    SECoP_V_destroy(&limited);
    return 0;
}
