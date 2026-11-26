#include <dunit.h>
#include <stdio.h>
#include <string.h>

struct M9State {
    int clicks;
};

static int32_t build(void *u, DunitUiNodeV1 *nodes, uint32_t cap) {
    (void)u;
    if (cap < 3) return -DUNIT_EINVAL;
    const char *title = "ROCK_C_TITLE";
    nodes[0] = (DunitUiNodeV1){DUNIT_UI_COLUMN, DUNIT_UI_ROOT, (const uint8_t *)"root", 4, (const uint8_t *)"", 0, 0};
    nodes[1] = (DunitUiNodeV1){DUNIT_UI_TEXT, 0, (const uint8_t *)"msg", 3, (const uint8_t *)title, strlen(title), 0};
    nodes[2] = (DunitUiNodeV1){DUNIT_UI_BUTTON, 0, (const uint8_t *)"ok", 2, (const uint8_t *)"OK", 2, 1};
    return 3;
}

static int32_t update(void *u, uint64_t msg) {
    struct M9State *s = u;
    if (msg == 1) {
        s->clicks++;
        puts("[M9-CGUI] action activated");
        return 1;
    }
    return 0;
}

int main(int argc, char **argv, char **envp) {
    setvbuf(stdout, NULL, _IONBF, 0);
    struct M9State st = {0};
    DunitUiCallbacksV1 cb = {DUNIT_UI_ABI_VERSION, sizeof cb, &st, build, update};
    puts("[M9-CGUI] ROCK_C_TITLE");
    return dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 380, 160, &cb);
}
