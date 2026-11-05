/* The only widget/layout/paint implementation is the Rust SDK archive. */
#include <dunit.h>
_Static_assert(sizeof(DunitUiNodeV1) == 48, "node ABI");
_Static_assert(sizeof(DunitUiCallbacksV1) == 32, "callback ABI");
struct Model {
    unsigned count;
    int invalid_tree, nested_checked, argc;
    char **argv, **envp;
    char label[32];
    DunitUiCallbacksV1 *callbacks;
};
static DunitUiNodeV1 node(uint32_t kind, uint32_t parent, const char *key,
                           const char *text, uint64_t message)
{
    DunitUiNodeV1 value = {kind, parent, (const uint8_t *)key, strlen(key),
        (const uint8_t *)text, strlen(text), message};
    return value;
}
static int32_t build(void *user, DunitUiNodeV1 *nodes, uint32_t capacity)
{
    struct Model *model = user;
    if (capacity < 4) return -DUNIT_EINVAL;
    nodes[0] = node(DUNIT_UI_COLUMN, DUNIT_UI_ROOT, "root", "", 0);
    snprintf(model->label, sizeof model->label, "Count %u", model->count);
    nodes[1] = node(DUNIT_UI_TEXT, model->invalid_tree ? 2 : 0, "title", model->label, 0);
    nodes[2] = node(DUNIT_UI_BUTTON, 0, "increment", "Increment", 10);
    nodes[3] = node(DUNIT_UI_BUTTON, 0, "quit", "Quit", 20);
    if (!model->nested_checked && !model->invalid_tree) {
        int result = dunit_ui_run_v1(model->argc, (const char *const *)model->argv,
            (const char *const *)model->envp, 400, 180, model->callbacks);
        if (result != -DUNIT_EBUSY) return -DUNIT_EINVAL;
        model->nested_checked = 1;
        puts("[CGUI] nested run rejected");
    }
    return 4;
}
static int32_t update(void *user, uint64_t message)
{
    struct Model *model = user;
    if (message == 20) return 1;
    if (message != 10) return -DUNIT_EINVAL;
    model->count++;
    if (model->count == 1) puts("[CGUI] count 1");
    if (model->count == 2) puts("[CGUI] count 2");
    return 0;
}
int main(int argc, char **argv, char **envp)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    struct Model model = {0};
    DunitUiCallbacksV1 callbacks = {DUNIT_UI_ABI_VERSION, sizeof callbacks, &model, build, update};
    model.argc = argc; model.argv = argv; model.envp = envp; model.callbacks = &callbacks;
    callbacks.abi_version++;
    if (dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 400, 180, &callbacks) != -DUNIT_EINVAL) return 1;
    callbacks.abi_version--;
    model.invalid_tree = 1;
    if (dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 400, 180, &callbacks) != -DUNIT_EINVAL) return 2;
    model.invalid_tree = 0;
    puts("[CGUI] bad version and invalid tree rejected");
    int result = dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 400, 180, &callbacks);
    if (result || model.count != 2 || !model.nested_checked) return 3;
    if (dunit_ui_run_v1(argc, (const char *const *)argv, (const char *const *)envp, 400, 180, &callbacks) != -DUNIT_EPIPE) return 4;
    puts("[CGUI] repeat run rejected");
    puts("[CGUI] ok");
    return 0;
}
