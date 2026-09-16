#include <EndpointSecurity/EndpointSecurity.h>
#include <stdio.h>

/* 只检查自己的后代客户端是否获准创建，不订阅事件、不请求或修改系统权限。 */
int main(void) {
    es_client_t *client = NULL;
    es_new_client_result_t result = es_new_descendants_client(
        &client, ^(es_client_t *client, const es_message_t *message) {});
    printf("%d\n", (int)result);
    if (result == ES_NEW_CLIENT_RESULT_SUCCESS) {
        es_delete_client(client);
    }
    return 0;
}
