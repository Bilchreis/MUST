/*
SPDX-License-Identifier: LGPL-3.0-or-later
Copyright (c) 2017-2019 Helmholtz-Zentrum Berlin fuer Materialien und Energie GmbH <https://www.helmholtz-berlin.de>
Modified for must (https://github.com/Bilchreis/MUST): Qt-free, C interface only;
the C++ class interface of CSECoPbaseType is not provided.
*/
#ifndef __SECOP_VARIANT_H__F003E0AD_31E3_4575_971F_F25EAD29F788__
#define __SECOP_VARIANT_H__F003E0AD_31E3_4575_971F_F25EAD29F788__

/**
 * \defgroup expfunc exported functions
 * \defgroup exptype exported definitions and types
 * \defgroup intfunc internal functions
 * \mainpage
 *    This library handles and stores SECoP data types and values.
 *    It has a C compatible interface with most of the functionality and
 *    a C++ interface for more convenient operation with data. If you
 *    are able to use the C++ interface, please do it.
 *
 *    The C interface checks the pointers for known object types, but
 *    it cannot prevent bugs of the caller.
 *
 *    See the GitHub web site for description of the SECoP protocol,
 *    definitions, data types and for formats:
 *
 *      https://github.com/SampleEnvironment/SECoP
 *
 *    The data here is stored only. The library will not automatically
 *    do things in background. There is a concept of ownership of memory,
 *    normally the library owns the memory or copies data. In special
 *    documented cases, the library takes ownership of the data, you
 *    do not have to free memory then.
 *
 * \code{.cpp}
 *    // create a simple integer array of 5 elements
 *    SECoP_V_create("{\"type\":\"array\",\"members\":{\"type\":\"int\"},\"maxlen\":5}");
 *    // create an tuple of an integer array of 5 elements and a string with 255 characters
 *    SECoP_V_create("{\"type\":\"tuple\",\"members\":[{\"type\":\"array\",\"members\":{\"type\":\"int\"},\"maxlen\":5},{\"type\":\"string\"}]}");
 *
 *    // use of SECoP_V_getInfo:
 *    unsigned int uPos = 0;
 *    SECoP_V_type iType;
 *    unsigned int uCount;
 *    const char* szName;
 *    double d;
 *    while (SECoP_V_getInfo(pData, uPos, &iType, &uCount, &szName))
 *    {
 *        if (szName != NULL)
 *            print("name=%s: ", szName);
 *        print("type=%u  count=%u\n", iType, uCount);
 *        if (iType == SECoP_VT_DOUBLE || iType == SECoP_VT_ARRAY_DOUBLE)
 *        {
 *            SECoP_V_getDouble(pData, uPos, 0, &d);
 *            print("a value=%f\n", d);
 *        }
 *        uPos = uPos + 1
 *    }
 *
 *    // use of SECoP_V_getInfo2:
 *    unsigned int uPos = 0;
 *    SECoP_V_type iType;
 *    unsigned int uCount, uLen;
 *    char szName[128];
 *    double d;
 *    uLen = sizeof(szName);
 *    while (SECoP_V_getInfo2(pData, uPos, &iType, &uCount, szName, &uLen))
 *    {
 *        if (uLen > 0)
 *            print("name=%.*s: ", uLen, szName);
 *        print("type=%u  count=%u\n", iType, uCount);
 *        if (iType == SECoP_VT_DOUBLE || iType == SECoP_VT_ARRAY_DOUBLE)
 *        {
 *            SECoP_V_getDouble(pData, uPos, 0, &d);
 *            print("a value=%f\n", d);
 *        }
 *        uPos = uPos + 1;
 *        uLen = sizeof(szName);
 *    }
 * \endcode
 */

#include <stdio.h>


//strcasecmp() is a function specified by posix, which MSVC does not support
#ifdef _MSC_VER
//not #if defined(_WIN32) || defined(_WIN64) because we have strncasecmp in mingw
#define strncasecmp _strnicmp
#define strcasecmp _stricmp
#endif

// SHALL_EXPORT is used to mark exported functions (needed by some compilers)
#if !defined(SHALL_EXPORT)
#  if defined(_MSC_VER)
#    define SHALL_EXPORT __declspec(dllimport)
#  else
#    define SHALL_EXPORT
#  endif
#endif

// the pointer which points to nothing
#ifndef NULL
#  ifdef __cplusplus
#    if __cplusplus >= 201103L
#      define NULL nullptr
#    else
#      define NULL 0
#    endif
#  else
#    define NULL ((void*)0)
#  endif
#endif

#ifdef _MSC_VER
#pragma warning(push)
#pragma warning(disable: 4661)
#endif

#ifdef __cplusplus
extern "C"
{
#endif

/**
 * \brief SECoP type codes and flags for variant type, \ref CSECoPbaseType
 * \ingroup exptype
 */
enum SECoP_V_type
{
    /** value is undefined */
    SECoP_VT_NONE          =  0,

    /** value is a floating point value */
    SECoP_VT_DOUBLE        =  1, // uCount == 1

    /** value is an integer */
    SECoP_VT_INTEGER       =  2, // uCount == 1

    /** value is a boolean */
    SECoP_VT_BOOL          =  3, // uCount == 1

    /** value is an enumeration (integer with named values) */
    SECoP_VT_ENUM          =  4, // uCount == 1

    /** value is a scaled integer */
    SECoP_VT_SCALED        =  5, // uCount == 1

    /** value is an array of a SECoP type (all values have same type) */
    SECoP_VT_ARRAY         = 10,

    /** value is an array of floating point values */
    SECoP_VT_ARRAY_DOUBLE  = 11,

    /** value is an array of integer values */
    SECoP_VT_ARRAY_INTEGER = 12,

    /** value is an array of boolean values */
    SECoP_VT_ARRAY_BOOL    = 13,

    /** value is an array of enumerations (integers with named values, all same type) */
    SECoP_VT_ARRAY_ENUM    = 14,

    /** value is an array of scaled integers (same multiplicator) */
    SECoP_VT_ARRAY_SCALED  = 15,

    /** value is a string */
    SECoP_VT_STRING        = 20,

    /** value is a binary object */
    SECoP_VT_BLOB          = 21,

    /** value is a JSON string */
    SECoP_VT_JSON          = 22,

    /** value is a structured type (key -> value of SECoP types) */
    SECoP_VT_STRUCT        = 30, // uCount > 0

    /** value is a fixed length array of different SECoP types */
    SECoP_VT_TUPLE         = 31, // uCount > 0

    /** value is a SECoP command, which may be special */
    SECoP_VT_COMMAND       = 40,
};

/** opaque handle of a SECoP value with its data type */
#if defined(__cplusplus)
class CSECoPbaseType;
#else
typedef struct CSECoPbaseType CSECoPbaseType;
#endif

/**
 * \brief result of SECoP_V_compare
 * \ingroup exptype
 */
enum SECoP_V_compareResult
{
    /** the two values are equal (equal types and equal values) */
    SECoP_VC_EQUAL = 0,

    /** the two values are similar (compatible types and equal values) */
    SECoP_VC_SIMILAR_VALUE = 1,

    /** the two values are different */
    SECoP_VC_DIFF_VALUE = 2,

    /** the two values have different types */
    SECoP_VC_DIFF_TYPE = 3
};


/**
 * \brief Function to create an pre-configured CSECoPbaseType structure from the
 *        description, which can be used in the SECoP client and server.
 *        Valid types are "double", "int", "bool", "string", "blob", "scaled", "enum",
 *        "array", "tuple" or "struct" like allowed data types in SECoP.
 * \ingroup expfunc
 * \param[in] szDescription structure of the data as described in SECoP standard
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_create(const char* szDescription);

/**
 * \brief Function to create a simple boolean value for convenience in SECoP.
 * \ingroup expfunc
 * \param[in] bValue new value to store
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_createSimpleBool(long long bValue);

/**
 * \brief Function to create a simple double value (64bit without limits) for convenience in SECoP.
 * \ingroup expfunc
 * \param[in] dValue new value to store
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_createSimpleDouble(double dValue);

/**
 * \brief Function to create a simple integer value (signed 64bit) for convenience in SECoP.
 * \ingroup expfunc
 * \param[in] llValue new value to store
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_createSimpleInteger(long long llValue);

/**
 * \brief Function to create a simple scaled integer value (signed 64bit) for convenience in SECoP.
 * \ingroup expfunc
 * \param[in] dScale scaling factor (>0.0)
 * \param[in] llValue new value to store
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_createSimpleScaled(double dScale, long long llValue);

/**
 * \brief Function to create a simple string value for convenience in SECoP.
 * \ingroup expfunc
 * \param[in] szValue new value to store
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_createSimpleString(const char* szValue);

/**
 * \brief Function to create a SECoP status for convenience in SECoP.
 *        This contains an enumeration (with 100=IDLE) and a string.
 *        {"type":"tuple","members":[{"type":"enum","members":{"IDLE":100}},{"type":"string"}]}
 * \ingroup expfunc
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_createSECoPStatus();

/**
 * \brief Use this function to get the SECoP data type of a CSECoPbaseType variant.
 * \ingroup expfunc
 * \param[in]  pData       input data
 * \param[out] szBuffer    buffer to write the SECoP data type description, if not NULL
 * \param[in]  iBufferSize size of szBuffer in bytes
 * \return negative for errors else the needed buffer size
 */
int SHALL_EXPORT SECoP_V_getType(const CSECoPbaseType* pData, char* szBuffer, int iBufferSize);

/**
 * \brief Use this function to convert SECoP-compatible data into a CSECoPbaseType variant.
 * \ingroup expfunc
 * \param[in] szJSON data to convert
 * \param[in] pHint  existing data to return a compatible type
 * \return a new CSECoPbaseType structure or NULL in case of errors
 */
CSECoPbaseType SHALL_EXPORT * SECoP_V_fromJSON(const char* szJSON, const CSECoPbaseType* pHint);

/**
 * \brief Use this function to convert a CSECoPbaseType variant into SECoP-compatible json data.
 * \ingroup expfunc
 * \param[in]  pData        input data to convert
 * \param[out] szJSONBuffer buffer to write the json data, if not NULL
 * \param[in]  iBufferSize  size of szJSONBuffer in bytes
 * \return negative for errors else the needed buffer size for the json data
 */
int SHALL_EXPORT SECoP_V_toJSON(const CSECoPbaseType* pData, char* szJSONBuffer, int iBufferSize);

/**
 * \brief Function to destroy an existing CSECoPbaseType structure with all its children.
 *        The storage pointer value is reset to NULL.
 * \ingroup expfunc
 * \param[in] ppData pointer to data to destroy
 */
void SHALL_EXPORT SECoP_V_destroy(CSECoPbaseType** ppData);

/**
 * \brief Function to copy existing CSECoPbaseType into a new
 * \ingroup expfunc
 * \param[out] ppDst destination (pointer will be allocated)
 * \param[in]  pSrc  source data
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_copy(CSECoPbaseType** ppDst, const CSECoPbaseType* pSrc);

/**
 * \brief print SECoP data to file using SECoP_V_printStream
 * \ingroup expfunc
 * \param[in] pOutfile   output stream to print to
 * \param[in] pData      input data to print
 * \param[in] iVerbosity 0=print data only, 1=print data with type, 2=print with additional info
 */
void SHALL_EXPORT SECoP_V_printFile(FILE* pOutfile, const CSECoPbaseType* pData, int iVerbosity);

/* SECoP_V_printString and SECoP_V_printStream use Qt types and are not provided. */

/**
 * \brief With this function you can get read access to any data stored in CSECoPbaseType.
 *        The position starts with zero and will go recursivly through the data. Every
 *        position has a data type and a number of values. When accessing a complex
 *        value, the first position is the complex data itself and returns a NULL data
 *        pointer. The next positions address the items of the complex data, while
 *        using recursive function calls to return the content of each item first,
 *        then the next item. A non-complex value has only one position regardless of
 *        the number of values. Access of a position outside all bounds gives a false
 *        return value.
 *        You may store this information to know the nesting level or structure of data.
 * \ingroup expfunc
 * \param[in]  pData     CSECoPbaseType with read access
 * \param[in]  uPosition zero-based position to data part
 * \param[out] piType    data type
 * \param[out] puCount   count of elements
 * \param[out] pszName   name of data
 * \return 1, if information was accessible; 0, if data or uPosition was invalid (end of data reached)
 */
int SHALL_EXPORT SECoP_V_getInfo(const CSECoPbaseType* pData, unsigned int uPosition, enum SECoP_V_type* piType,
                                 unsigned int* puCount, const char** pszName);

/**
 * \brief With this function you can get read access to any data stored in CSECoPbaseType.
 *        The position starts with zero and will go recursivly through the data. Every
 *        position has a data type and a number of values. When accessing a complex
 *        value, the first position is the complex data itself and returns a NULL data
 *        pointer. The next positions address the items of the complex data, while
 *        using recursive function calls to return the content of each item first,
 *        then the next item. A non-complex value has only one position regardless of
 *        the number of values. Access of a position outside all bounds gives a false
 *        return value.
 *        You may store this information to know the nesting level or structure of data.
 * \ingroup expfunc
 * \param[in]     pData     CSECoPbaseType with read access
 * \param[in]     uPosition zero-based position to data part
 * \param[out]    piType    data type
 * \param[out]    puCount   count of elements
 * \param[out]    szName    name buffer
 * \param[in,out] puLen     size of "szName" buffer (in), name length (out)
 * \return 1, if information was accessible; 0, if data or uPosition was invalid (end of data reached)
 */
int SHALL_EXPORT SECoP_V_getInfo2(const CSECoPbaseType* pData, unsigned int uPosition, enum SECoP_V_type* piType,
                                  unsigned int* puCount, char* szName, unsigned int* puLen);

/**
 * \brief Set minimum and maximum double value.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData      CSECoPbaseType to read
 * \param[in] uPosition  zero-based position to data part
 * \param[in] dblMinimum minimum value (or NaN)
 * \param[in] dblMaximum maximum value (minimum <= maximum or NaN)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyMinMaxDouble(CSECoPbaseType* pData, unsigned int uPosition,
                                            double dblMinimum, double dblMaximum);

/**
 * \brief Set minimum and maximum integer value.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to modify
 * \param[in] uPosition zero-based position to data part
 * \param[in] llMinimum minimum value
 * \param[in] llMaximum maximum value (minimum <= maximum)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyMinMaxInteger(CSECoPbaseType* pData, unsigned int uPosition,
                                             long long llMinimum, long long llMaximum);

/**
 * \brief Set current size, minimum size and maximum size of array with the constraint: minimum <= current <= maximum
 *        a special call is available: if minimum size == 1 and maximum size == 0, only the current size is changed and
 *        the minimum and maximum sizes are ignored. The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData          CSECoPbaseType to read
 * \param[in] uPosition      zero-based position to data part
 * \param[in] uLength        current size (minimum <= current <= maximum)
 * \param[in] uMinimumLength minimum array size (>=0)
 * \param[in] uMaximumLength maximum array size (minimum <= maximum)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyArrayLength(CSECoPbaseType* pData, unsigned int uPosition, unsigned int uLength,
                                           unsigned int uMinimumLength, unsigned int uMaximumLength);

/**
 * \brief Use this function to modify a specific double (numeric) value.
 *        This item is selected via its uPosition (\ref SECoP_V_getInfo) and must
 *        have the type SECoP_VT_DOUBLE. You cannot extend or shrink the array size.
 *        The data pointer is used but not modified - only the data itself is modified.
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to modify
 * \param[in] uPosition zero-based position to data part
 * \param[in] uIndex    zero-based array index into data part
 * \param[in] dblValue  new value
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyDouble(CSECoPbaseType* pData, unsigned int uPosition,
                                      unsigned int uIndex, double dblValue);

/**
 * \brief Use this function to modify a specific integer (numeric) value.
 *        This item is selected via its uPosition (\ref SECoP_V_getInfo) and must
 *        have the type SECoP_VT_INTEGER, SECoP_VT_BOOL, SECoP_VT_ENUM or SECoP_VT_SCALED.
 *        You cannot extend or shrink the array size.
 *        The data pointer is used but not modified - only the data itself is modified.
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to modify
 * \param[in] uPosition zero-based position to data part
 * \param[in] uIndex    zero-based array index into data part
 * \param[in] llValue   new value
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyInteger(CSECoPbaseType* pData, unsigned int uPosition,
                                       unsigned int uIndex, long long llValue);

/**
 * \brief Use this function to modify the factor of a scaled integer.
 *        This item is selected via its uPosition (\ref SECoP_V_getInfo) and must
 *        have the type SECoP_VT_SCALED. You cannot extend or shrink the array size.
 *        The data pointer is used but not modified - only the data itself is modified.
 * \ingroup expfunc
 * \param[in,out] pData          CSECoPbaseType to modify
 * \param[in]     uPosition      zero-based position to data part
 * \param[in]     dblScaleFactor new scaling factor (example: integer=999, scale=0.1 -> means 99.9 for transport)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyScale(CSECoPbaseType* pData, unsigned int uPosition,
                                     double dblScaleFactor);

/**
 * \brief Use this function to replace existing string data inside the SECoP variant.
 *        This string is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in,out] pData     CSECoPbaseType to modify
 * \param[in]     uPosition zero-based position to SECoP_VT_STRING, SECoP_VT_BLOB or SECoP_VT_JSON
 * \param[in]     szValue   string value to replace
 * \param[in]     iSize     length of string in bytes or -1 for null-terminated string
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_modifyString(CSECoPbaseType* pData, unsigned int uPosition,
                                      const char* szValue, int iSize);

/**
 * \brief Read minimum and maximum array or data sizes.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData           CSECoPbaseType to read
 * \param[in]  uPosition       zero-based position to data part
 * \param[out] puLength        current array size (minimum <= current <= maximum)
 * \param[out] puMinimumLength minimum array size (>=0)
 * \param[out] puMaximumLength maximum array size (minimum <= maximum)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_getArrayLength(const CSECoPbaseType* pData, unsigned int uPosition,
                                        unsigned int* puLength, unsigned int* puMinimumLength,
                                        unsigned int* puMaximumLength);

/**
 * \brief Read minimum and maximum double value.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData       CSECoPbaseType to read
 * \param[in]  uPosition   zero-based position to data part
 * \param[out] pdblMinimum minimum value (or NaN)
 * \param[out] pdblMaximum maximum value (minimum <= maximum or NaN)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_getMinMaxDouble(const CSECoPbaseType* pData, unsigned int uPosition,
                                         double* pdblMinimum, double* pdblMaximum);

/**
 * \brief Read minimum and maximum integer value.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData       CSECoPbaseType to read
 * \param[in]  uPosition   zero-based position to data part
 * \param[out] pllMinimum minimum value
 * \param[out] pllMaximum maximum value (minimum <= maximum)
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_getMinMaxInteger(const CSECoPbaseType* pData, unsigned int uPosition,
                                          long long* pllMinimum, long long* pllMaximum);

/**
 * \brief Use this function to read a single double value out of the SECoP variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData     CSECoPbaseType to read
 * \param[in]  uPosition zero-based position to data part (complete array)
 * \param[in]  uIndex    zero-based index into double array
 * \param[out] pdblValue memory location which will get the double value
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_getDouble(const CSECoPbaseType* pData, unsigned int uPosition,
                                   unsigned int uIndex, double* pdblValue);

/**
 * \brief Use this function to read a single integer value out of the SECoP variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData     CSECoPbaseType to read
 * \param[in]  uPosition zero-based position to data part (complete array)
 * \param[in]  uIndex    zero-based index into integer array
 * \param[out] pllValue  memory location which will get the integer value
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_getInteger(const CSECoPbaseType* pData, unsigned int uPosition,
                                    unsigned int uIndex, long long* pllValue);

/**
 * \brief Use this function to read a string/blob value out of the SECoP variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData       CSECoPbaseType to read
 * \param[in]  uPosition   zero-based position to data part
 * \param[in]  uBufferSize size of memory location (function will not copy more than this number of bytes)
 * \param[out] pszValue    memory location which will get the string value
 * \return zero for errors, number of bytes needed (including a trailing NUL)
 */
unsigned int SHALL_EXPORT SECoP_V_getString(const CSECoPbaseType* pData, unsigned int uPosition,
                                            unsigned int uBufferSize, char* pszValue);

/**
 * \brief Function to read the factor of a scaled integer out of the SECoP variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData     CSECoPbaseType to read
 * \param[in]  uPosition zero-based position to data part
 * \param[out] pdblValue memory location which will get the scale factor
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_getScale(const CSECoPbaseType* pData, unsigned int uPosition,
                                  double* pdblValue);

/**
 * \brief Return count of enumeration items in list or structure names.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to read
 * \param[in] uPosition zero-based position to data part
 * \return return count of enumeration items in list, structure names or 0 on error
 */
unsigned int SHALL_EXPORT SECoP_V_getEnumStructCount(const CSECoPbaseType* pData, unsigned int uPosition);

/**
 * \brief Return enumeration item from list with zero based index.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData      CSECoPbaseType to read
 * \param[in]  uPosition  zero-based position to data part
 * \param[in]  uIndex     zero based index to enum item list
 * \param[out] pllEnumVal enumeration value for valid index or NULL
 * \return for valid index: enumeration name, for invalid index: NULL
 */
const char SHALL_EXPORT * SECoP_V_getEnumeration(const CSECoPbaseType* pData, unsigned int uPosition,
                                                 unsigned int uIndex, long long* pllEnumVal);

/**
 * \brief Return enumeration item from list with zero based index.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData      CSECoPbaseType to read
 * \param[in]  uPosition  zero-based position to data part
 * \param[in]  llEnumVal  enumeration value
 * \param[in]  szEnumName enumeration name
 * \return 0 on error, 1 on success
 */
int SHALL_EXPORT SECoP_V_putEnumeration(CSECoPbaseType* pData, unsigned int uPosition,
                                        long long llEnumVal, const char* szEnumName);

/**
 * \brief Return name or struct item from any variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to read
 * \param[in] uPosition zero-based position to data part
 * \return for valid index: struct or dictionary name, for invalid index: NULL
 */
const char SHALL_EXPORT * SECoP_V_getStructName(const CSECoPbaseType* pData, unsigned int uPosition);

/**
 * \brief Compare two values and check for equal/compatible types and equal values.
 * \ingroup expfunc
 * \param[in] pData1 first value
 * \param[in] pData2 second value
 * \return result \ref SECoP_V_compareResult
 */
enum SECoP_V_compareResult SHALL_EXPORT SECoP_V_compare(const CSECoPbaseType* pData1, const CSECoPbaseType* pData2);

/**
 * \brief Retrieve number of additional items of variant (e.g. unit).
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to read
 * \param[in] uPosition zero-based position to data part
 * \return number of additional items or zero
 */
unsigned int SHALL_EXPORT SECoP_V_getAdditionalCount(const CSECoPbaseType* pData, unsigned int uPosition);

/**
 * \brief Retrieve index of additional item of a variant by name.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to read
 * \param[in] uPosition zero-based position to data part
 * \param[in] szName    name of item
 * \return index to additional items or UINT_MAX (-1 as invalid index)
 */
unsigned int SHALL_EXPORT SECoP_V_getAdditionalIndex(const CSECoPbaseType* pData, unsigned int uPosition, const char* szName);

/**
 * \brief Retrieve the key name of additional (string,json-array,json-object) items of a variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData       CSECoPbaseType to read
 * \param[in]  uPosition   zero-based position to data part
 * \param[in]  uIndex      zero-based index to additional item (\ref SECoP_V_getAdditionalCount)
 * \param[in]  uBufferSize size of memory location (function will not copy more than this number of bytes)
 * \param[out] pszValue    memory location which will get the string value
 * \return zero for errors, number of bytes needed (including a trailing NUL)
 */
unsigned int SHALL_EXPORT SECoP_V_getAdditionalKey(const CSECoPbaseType* pData, unsigned int uPosition, unsigned int uIndex,
                                                   unsigned int uBufferSize, char* pszValue);

/**
 * \brief Retrieve a string of additional (string,json-array,json-object) items of a variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in]  pData       CSECoPbaseType to read
 * \param[in]  uPosition   zero-based position to data part
 * \param[in]  uIndex      zero-based index to additional item (\ref SECoP_V_getAdditionalCount)
 * \param[in]  uBufferSize size of memory location (function will not copy more than this number of bytes)
 * \param[out] pszValue    memory location which will get the string value
 * \return zero for errors, number of bytes needed (including a trailing NUL)
 */
unsigned int SHALL_EXPORT SECoP_V_getAdditionalString(const CSECoPbaseType* pData, unsigned int uPosition, unsigned int uIndex,
                                                      unsigned int uBufferSize, char* pszValue);

/**
 * \brief Retrieve the numeric value as double of additional items of a variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to read
 * \param[in] uPosition zero-based position to data part
 * \param[in] uIndex    zero-based index to additional item (\ref SECoP_V_getAdditionalCount)
 * \return on success the value, NaN on errors
 */
double SHALL_EXPORT SECoP_V_getAdditionalDouble(const CSECoPbaseType* pData, unsigned int uPosition, unsigned int uIndex);

/**
 * \brief Retrieve the numeric value as integer of additional items of a variant.
 *        The item is selected via its uPosition (\ref SECoP_V_getInfo).
 * \ingroup expfunc
 * \param[in] pData     CSECoPbaseType to read
 * \param[in] uPosition zero-based position to data part
 * \param[in] uIndex    zero-based index to additional item (\ref SECoP_V_getAdditionalCount)
 * \return on success the value, zero on errors (with might be a valid value)
 */
long long SHALL_EXPORT SECoP_V_getAdditionalInteger(const CSECoPbaseType* pData, unsigned int uPosition, unsigned int uIndex);

#ifdef __cplusplus
} // end: extern "C"
#endif

#ifdef _MSC_VER
#pragma warning(pop)
#endif

#endif /*__SECOP_VARIANT_H__F003E0AD_31E3_4575_971F_F25EAD29F788__*/
