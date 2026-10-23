# Science Tokyo LMS API

[OpenAPI 3.0.3](openapi.yaml) · [Swagger UI](index.html) ·
[实测记录](verification.json) · [接入评估](../lms-token-feasibility.md)

这份协议覆盖 IsctCompanion 接入需要、并已在 2026-10-20 验证的 8 个读取函数，
以及公开配置、官方浏览器授权和资源文件下载。学校服务声明了 470 个函数；
这里没有把其余函数标为已验证，也不包含提交、上传等写操作的调用协议。

在仓库根目录启动查看器：

```sh
python3 -m http.server 8766 --bind 127.0.0.1 --directory docs/lms-api
```

打开 `http://127.0.0.1:8766/`。查看器从同目录读取 `openapi.yaml`，使用固定版本、
带完整性校验的 Swagger UI 5.34.0；首次加载需要访问 jsDelivr。也可以将 YAML
导入自己的 Swagger/OpenAPI 工具。查看器用于阅读，关闭了在线执行和远程校验。

## 已验证的原生传输

- 使用与项目一致的 reqwest 0.12.28、`rustls-tls`，保留证书校验。
- 使用项目下载客户端现有的
  `Mozilla/5.0 (compatible; IsctCompanion/0.1)` User-Agent。
- 不启用 Cookie jar，不发送浏览器 Cookie，也不使用 `sesskey`。
- REST 使用 POST 表单，关闭自动重定向；token 放在表单中。
- 默认 curl 标识的请求得到 AWS 网关 403，而上述应用标识能成功。这是本次对照
  观察，不能据此推定学校完整的网关匹配规则。

8 个函数均通过了原生 POST，实际响应与文档中的响应模型逐一匹配。模型列出
接入相关字段并允许额外字段；不是 Moodle 所有权限组合和模块插件的完整类型定义。

## 真实请求格式

根地址是 `https://lms.s.isct.ac.jp/2025`；路径中的 2025 也承载当前的 2026 课程。
Moodle 使用单一路径的 RPC，不存在本文虚构的 `/courses` 或 `/assignments` REST 路由：

```http
POST /2025/webservice/rest/server.php HTTP/1.1
Host: lms.s.isct.ac.jp
User-Agent: Mozilla/5.0 (compatible; IsctCompanion/0.1)
Content-Type: application/x-www-form-urlencoded

wstoken=REPLACE_WITH_TOKEN&wsfunction=mod_assign_get_assignments&moodlewsrestformat=json&courseids%5B0%5D=1001
```

示例中的编号均为合成数据。使用表单编码库编码整个请求：`courseids[0]`、
`courseids[1]` 是实际字段名，不要把数组编码成逗号字符串，也不发送 JSON 请求体。
OpenAPI 的 `RpcRequest.oneOf` 和 `wsfunction` discriminator 描述各函数的请求分支；
响应分支见 `x-moodle-functions` 的 `responseSchema`。

| 函数 | 关键输入 | 输出 |
|---|---|---|
| `core_webservice_get_site_info` | 无额外参数 | 本人 userid、站点版本、此服务的能力列表 |
| `core_enrol_get_users_courses` | 本人 userid | 课程数组；实测 62 门 |
| `core_block_get_dashboard_blocks` | returncontents=1 | 默认课表区块；2026-3Q 实测与手机数据一致 |
| `core_course_get_contents` | courseid | 章节、HTML、模块、文件清单 |
| `mod_assign_get_assignments` | courseids[n] | 作业实例、截止时间、说明、附件 |
| `mod_assign_get_submission_status` | assignid | 本人的真实提交状态和评分反馈 |
| `mod_forum_get_forum_discussions` | forumid；可显式传 page/perpage | 讨论及首帖正文 |
| `core_calendar_get_action_events_by_timesort` | 时间范围、limitnum、aftereventid | 行动事件及分页游标 |
| `tool_mobile_get_config` | section=mobileapp | 移动端策略键值对 |

`Module.id` 是 cmid，`Module.instance` 才是 assignid/forumid。
`Assignment.id` 是 assignid，`Assignment.cmid` 是 cmid。
讨论列表的 `discussion` 是讨论 ID，`id` 是首帖 ID。
时间戳以秒为单位；0 通常表示未设置。HTML 通过应用现有的内容处理后再显示。

日常刷新现在优先用 token 同步课程列表和 `tttimetable` 默认课表。区块的
`contents.content` 交给既有 Rust 课表解析器，未返回的历史季度保留缓存。
区块接口不可用或课表无法解析时，回退到既有 Cookie 读取路径；网络故障按原策略重试。

跨季度的 `year`、`quota`、`search` 参数会被 REST 拒绝（`invalidparameter`）。
学校没有公开课表专用服务函数，也未在移动插件列表中提供 `block_tttimetable`。
既有网页入口是登录后的 `POST /2025/my/index.php`；本次没有新增修改用户偏好的接口。
完整 Core 的后台实测同步了 62 门课程，Cookie 端口和浏览器导航调用均为 0。

## 文件下载

使用 `contents[].fileurl` 的原始路径和参数，在确认同源且路径属于学校文件接口后，
添加查询参数 `token`。不要把它改成 REST 的 `wstoken`，也不要向其他源转发。
OpenAPI 的文件路径模板只描述已验证的 `mod_resource` 类型；其他模块使用返回 URL，
不要自行拼装。HTTP 客户端错误对象中的 URL 也要去掉 token 后才能记录。

本次无 Cookie 的原生下载得到：

- 完整请求：HTTP 200、`application/pdf`、45,268 字节，与元数据一致；
  文件以 `%PDF-` 开头，末尾有 `%%EOF`。
- Range：`bytes=0-1023` 返回 HTTP 206、`Content-Range: bytes 0-1023/45268`，
  共 1,024 字节，与完整文件前缀一致。
- 错误 token：HTTP 200，但正文为包含 `invalidtoken` 的 JSON，不是 PDF。

因此，`status == 200` 不能单独证明下载成功。检查内容类型、大小和范围语义；
根据文件类型校验实际内容。上述检查不代表验证了所有文件类型、超大文件或服务器变更后的行为。

## Token 生命周期

实测 `tool_mobile_forcelogout=0`、`tool_mobile_autologout=0`；
`tool_mobile_autologouttime=86400` 是关闭状态下的策略值，不能当作 token TTL。
同一枚 token 跨本次调查的多次原生请求仍然有效，且每次都不携带浏览器 Cookie。

Moodle 上游的官方移动端流程使用永久类型 token：新建 token 时不绑定网页登录会话，
并将到期时间设置为创建时间加站点 `tokenduration`。上游默认是 12 周，但学校可以修改，
而且授权入口可能返回已有 token。因此，**当前 token 的创建时间和具体到期时间未知**；
不能从本次取得它的时间推算。正常 API 请求更新 lastaccess，不自动延长 validuntil。
参见[上游令牌生成实现](https://github.com/moodle/moodle/blob/MOODLE_405_STABLE/lib/external/classes/util.php)。

本次没有让网页登录会话实际过期，也没有等待 token 自然到期或执行撤销。
学校修改、补丁和管理员策略仍可能影响生命周期。接入时将到期时间视为未知，
识别 `invalidtoken`、`webservice_access_exception` 等认证失败后暂停请求，
保留已缓存数据，并重新走官方浏览器授权；不要无限重试或虚构 refresh_token。
`privatetoken` 用于其他移动端流程，不是 refresh_token，本次也没有取得它。

## 错误与范围

REST 的无效 token 实测也是 HTTP 200，JSON 带 `exception`、`errorcode`、`message`。
必须先识别错误 envelope，再按 wsfunction 解析成功数据。网关 403 可能是 HTML；
与 Moodle 的业务错误分开处理。`warnings` 表示可能只有部分结果，不能在不完整响应后
删除本地旧数据。未遍历所有课程、权限和错误码。

此移动服务的 token 有写权限，客户端必须单独维护读取函数白名单。学校自定义时间表、
外部嵌入活动和完整内容一致性尚未验证，不应一次移除现有网页登录和补充读取。

需要真实账户的复核登记为 `SCHOOL-LMS-TOKEN-HTTP`；证据文件不包含 token、
账号资料或课程正文。应用尚未切换到此协议，生产代码没有变更。
