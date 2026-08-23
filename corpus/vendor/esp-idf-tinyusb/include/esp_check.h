#pragma once

#define ESP_RETURN_ON_FALSE(condition, error, tag, message) \
    do {                                                     \
        (void)(tag);                                         \
        if (!(condition)) {                                  \
            return (error);                                  \
        }                                                    \
    } while (0)

#define ESP_RETURN_ON_ERROR(expression, tag, message) \
    do {                                               \
        (void)(tag);                                   \
        esp_err_t check_error = (expression);          \
        if (check_error != ESP_OK) {                   \
            return check_error;                        \
        }                                              \
    } while (0)

#define ESP_GOTO_ON_ERROR(expression, label, tag, message) \
    do {                                                 \
        (void)(tag);                                     \
        ret = (expression);                              \
        if (ret != ESP_OK) {                             \
            goto label;                                  \
        }                                                \
    } while (0)

#define ESP_GOTO_ON_FALSE(condition, error, label, tag, message) \
    do {                                                    \
        (void)(tag);                                        \
        if (!(condition)) {                                 \
            ret = (error);                                  \
            goto label;                                     \
        }                                                   \
    } while (0)
