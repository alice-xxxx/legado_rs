#ifndef LEGADO_SOURCE_ENGINE_HOST_H
#define LEGADO_SOURCE_ENGINE_HOST_H

#ifdef __cplusplus
extern "C" {
#endif

/*
 * 移动端 KMP 通过此稳定 C 边界提交 JSON 请求。
 * Rust 负责网络和应用数据存储，Kotlin 负责书源规则执行。
 * 响应为 UTF-8 JSON：{"ok":true,"value":...} 或 {"ok":false,"error":"..."}。
 * 所有返回指针（包括错误响应）都必须交给对应 free 函数释放。
 */
char *legado_source_host_http(const char *request_json);
char *legado_source_host_storage(const char *request_json, const char *app_data_dir);
char *legado_source_host_image(const char *request_json);
void legado_source_host_string_free(char *value);

#ifdef __cplusplus
}
#endif

#endif
