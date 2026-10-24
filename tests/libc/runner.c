/* Dunit adapter, deliberately separate from unmodified upstream test sources. */
#include <stdio.h>
#include "test.h"
#ifndef DUNIT_LIBC_TEST_NAME
#error DUNIT_LIBC_TEST_NAME must identify the original upstream case
#endif
int libc_test_case_main(void);
int main(void)
{
    printf("[LIBC-TEST] %s BEGIN\n", DUNIT_LIBC_TEST_NAME);
    fflush(stdout);
    int result = libc_test_case_main();
    int failed = result != 0 || t_status != 0;
    printf("[LIBC-TEST] %s %s rc=%d status=%d\n",
           DUNIT_LIBC_TEST_NAME, failed ? "FAIL" : "PASS", result, t_status);
    return failed ? 1 : 0;
}
