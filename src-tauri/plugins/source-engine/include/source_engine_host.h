#ifndef FOLIVER_SOURCE_ENGINE_HOST_H
#define FOLIVER_SOURCE_ENGINE_HOST_H
#ifdef __cplusplus
extern "C" {
#endif
/* 返回字符串必须由 foliver_source_free 释放。 */
char *foliver_source_http(const char *input);
char *foliver_source_storage(const char *input);
char *foliver_source_image(const char *input);
char *foliver_source_browser(const char *input);
int foliver_source_cancelled(const char *id);
void foliver_source_free(char *value);
#ifdef __cplusplus
}
#endif
#endif
