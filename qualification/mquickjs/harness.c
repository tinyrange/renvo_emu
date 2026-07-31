#include <stddef.h>
#include <stdint.h>

#include "mquickjs.h"

static JSValue js_print(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)ctx;
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_UNDEFINED;
}

static JSValue js_gc(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)this_val;
    (void)argc;
    (void)argv;
    JS_GC(ctx);
    return JS_UNDEFINED;
}

static JSValue js_load(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)ctx;
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_UNDEFINED;
}

static JSValue js_setTimeout(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)ctx;
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_NewInt32(ctx, 0);
}

static JSValue js_clearTimeout(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)ctx;
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_UNDEFINED;
}

JSValue js_date_constructor(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_NewDate(ctx, 0);
}

static JSValue js_date_now(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_NewInt32(ctx, 0);
}

static JSValue js_performance_now(JSContext *ctx, JSValue *this_val, int argc, JSValue *argv)
{
    (void)this_val;
    (void)argc;
    (void)argv;
    return JS_NewInt32(ctx, 0);
}

#include "mqjs_stdlib.h"

#ifndef RENVO_RUNTIME_KB
#define RENVO_RUNTIME_KB 160
#endif
#ifndef RENVO_ERROR_OFFSET
#define RENVO_ERROR_OFFSET 0
#endif

static uint8_t runtime_memory[RENVO_RUNTIME_KB * 1024] __attribute__((aligned(16)));

extern uint8_t renvo_mquickjs_bytecode_start[];
extern uint8_t renvo_mquickjs_bytecode_end[];

int main(void)
{
    JSContext *ctx = JS_NewContext(runtime_memory, sizeof(runtime_memory), &js_stdlib);
    if (ctx == NULL) {
        return 10;
    }

    uint32_t bytecode_length =
        (uint32_t)(renvo_mquickjs_bytecode_end - renvo_mquickjs_bytecode_start);
    if (JS_RelocateBytecode(ctx, renvo_mquickjs_bytecode_start, bytecode_length) != 0) {
        return 13;
    }
    JSValue result = JS_LoadBytecode(ctx, renvo_mquickjs_bytecode_start);
    if (!JS_IsException(result)) {
        result = JS_Run(ctx, result);
    }
    if (JS_IsException(result)) {
        JSCStringBuf buffer;
        const char *message = JS_ToCString(ctx, JS_GetException(ctx), &buffer);
        uint32_t diagnostic = 0;
        if (message != NULL) {
            for (unsigned index = 0;
                 index < 4 && message[RENVO_ERROR_OFFSET + index] != '\0';
                 ++index) {
                diagnostic |= (uint32_t)(uint8_t)message[RENVO_ERROR_OFFSET + index]
                    << (index * 8);
            }
        }
        return diagnostic == 0 ? 11 : (int)diagnostic;
    }

    int value = -1;
    if (JS_ToInt32(ctx, &value, result) != 0) {
        return 12;
    }
    if (value != 0) {
        return 20 + value;
    }
    JS_FreeContext(ctx);
    return 0;
}
