# Science Tokyo LMS API

[OpenAPI 3.0.3](openapi.yaml) · [Swagger UI](index.html) ·
[实测记录](verification.json) · 接入评估：slotledger `approach/lms-token-rest-transport`

这份协议覆盖 IsctCompanion 接入需要、并已在 2026-10-20 验证的 8 个读取函数，
以及公开配置、官方浏览器授权和资源文件下载。学校服务声明了 470 个函数；
这里没有把其余函数标为已验证。[写入面调查](#写入面调查2026-10-31)
列出了提交相关的写函数、作业设置和可撤回性，并在一个作业上做过一次
「上传 → 保存 → 撤回」实测；写入协议只验证了这一种配置。

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

课程表分两种读取，由同步规划器决定（`sync_engine::Depth`）：

- 轻量（自动触发）：用 token 读课程列表和 `tttimetable` 默认课表（当前季度）。区块的
  `contents.content` 交给既有 Rust 课表解析器，未返回的季度保留缓存。失败时不回退，
  按原策略等下次重试。
- 全量（刷新按钮，或该键第一次读取）：课程列表照常读，所有季度的课表都走 Cookie
  的 `POST /2025/my/index.php`，因为 token 只能看到当前季度。

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
账号资料或课程正文。应用的 LMS 读取已经使用这套协议：Core 的 `TokenTransport`
（`crates/app-core/src/net/moodle/`）为主，token 路径不可用时才由 Cookie 传输接管。

## 写入面调查（2026-10-31）

为闭合的写入枚举和 Core 的 `SubmissionCapabilities` 做的调查（slotledger
`lms-write-api-survey`）。先做了只读调查；之后经用户同意，在一个作业上做了一次
保存再撤回的[写入实测](#写入实测经用户同意)，没有开始小测。下文把「本实例实测」
和「上游源码行为」分开写：后者读自 Moodle `v4.5.10` 标签（学校为 4.5.10+，
Build 20260227；相关文件与 4.5 分支最新版只差评分分配的权限检查和一个表单标签），
标「已实测」的条目之外都只是推断。

探针是被忽略的实时测试 `write_surface_survey`
（`crates/app-core/src/tests/net__moodle__rest.rs`）。它从登录钥匙串读取桌面端
已存的 token（macOS 会请用户确认），所有账户请求都经 Core 的
`TokenTransport::call` 发出；这个接口只接受只读的 `MoodleFunction`，探针里写不出
写调用，token 失效时也不会重新授权。读取了 `core_webservice_get_site_info`、
`core_course_get_enrolled_courses_by_timeline_classification`、
`mod_assign_get_assignments`、`mod_assign_get_submission_status`、
`mod_quiz_get_quizzes_by_courses`、`mod_quiz_get_user_attempts` 和
`mod_quiz_get_quiz_access_information`（为读取口令和 SEB 规则新加入只读枚举）。
2026-10-31 那次运行还发了一次不带 token、不带文件的
`GET /2025/webservice/upload.php`；这一步之后从探针中删除，仓库里不留调用上传端点的
代码。汇总只含代码、计数和设置值，编号均为合成值，记在
[verification.json](verification.json) 的 `writeSurfaceSurvey`。

### 服务开放的写函数

`core_webservice_get_site_info` 仍列出 470 个函数，并返回 `uploadfiles=1`、
`downloadfiles=1`、`usermaxuploadfilesize=262144000`（250 MiB）、
`userquota=104857600`（100 MiB，私人文件配额）、`usercanmanageownfiles=true`。
不带 token 的 `GET webservice/upload.php` 得到 HTTP 200 和 JSON
`errorcode=missingparam`：网关放行这条路径，Moodle 在认证前拒绝。结合
`uploadfiles=1`，上传入口对此服务开放；实际上传见写入实测。

与学生提交有关、此服务开放的写函数（上游均为 `type=write`）：

| 函数 | 参数 | 返回 | 作用 |
|---|---|---|---|
| `webservice/upload.php`（不是 REST 函数） | multipart：`token`（不是 `wstoken`）、`filepath`（默认 `/`）、`itemid`（0 = 新建草稿区）、任意文件字段 | 每个文件一条记录（含 `itemid`），或 `errortype` 为 `fileoversized`、`filenameexist` | 上传到本人的草稿区 |
| `core_files_get_unused_draft_itemid` | 无 | `component`、`contextid`、`userid`、`filearea`、`itemid`、`warnings` | 取一个未用的草稿 itemid；只生成编号，不写库 |
| `core_files_delete_draft_files` | `draftitemid`、`files[n][filepath]`、`files[n][filename]` | `parentpaths`、`warnings` | 删除草稿区中的文件 |
| `mod_assign_save_submission` | `assignmentid`；`plugindata[files_filemanager]`=草稿 itemid；`plugindata[onlinetext_editor][text]`、`[format]`、`[itemid]` | `warnings` | 保存提交；本站等于提交 |
| `mod_assign_submit_for_grading` | `assignmentid`、`acceptsubmissionstatement`（bool） | `warnings` | 草稿改为已提交；本站作业都不需要 |
| `mod_assign_remove_submission` | `userid`（本人）、`assignid` | `status`、`warnings` | 删除本次提交的全部内容 |
| `mod_assign_start_submission` | `assignid` | `submissionid`、`warnings` | 开始计时作业的计时 |

`mod_assign_view_assign` 和 `mod_assign_view_submission_status` 也是
`type=write`，但只记录查看事件和完成度，提交不需要。`core_files_upload`
不在此服务中（上游也不把它放进移动服务）；没有让学生复制上次尝试或新开尝试的函数。

不进入写入枚举的函数：

- 教师函数：`mod_assign_save_grade(s)`、`lock_submissions`、`unlock_submissions`、
  `revert_submissions_to_draft`、`save_user_extensions`、`submit_grading_form`、
  `set_user_flags`、`reveal_identities`，学生没有相应权限。
- 小测作答：`mod_quiz_start_attempt`、`save_attempt`、`process_attempt` 及
  `view_*`，按 slotledger 决定不在范围内。
- `mod_ttws_*`（19 个）：没有公开源码。去掉前缀后与 `mod_workshop_*` 的 19 个函数
  逐一同名（`add/update/delete_submission`、`evaluate_*`、`update_assessment` 等），
  推断是互评模块的改版；按课程评价处理，不在范围内，参数未知。
- `tool_mobile_call_external_functions`：上游 `type=write`，可在一次请求里执行同一
  服务的任何函数。允许它就等于没有白名单，读写两个枚举都不能包含它。
- 私人文件（`core_user_add_user_private_files`、`prepare_private_files_for_edition`、
  `update_private_files`）和 `core_comment_add_comments`、`delete_comments`：
  提交不需要。

### 本实例实测

46 门课程：2026-1Q 15、2Q 11、3Q 11、4Q 9。`mod_assign_get_assignments` 对 9 门课程
返回 warningcode `1`（无访问权限）。83 个作业全部读取了设置和提交状态，其中本学季
3Q 和上学季 2Q 共 32 个，2026-1Q 的 51 个作为对照。83 个作业的设置如下：

- `submissiondrafts=0`：没有「草稿 → 提交」两步，保存即为 `submitted`；
  `cansubmit` 全为 false。
- `requiresubmissionstatement=0`，没有返回声明文本。
- `teamsubmission=0`、`timelimit=0`、`blindmarking=0`、`markingworkflow=0`、
  `sendnotifications=0`；`nosubmissions=1` 只有 1Q 的 1 个线下作业。
- 没有延期、锁定或以前的尝试：`extensionduedate` 未设，`locked=false`，
  `attemptnumber=0`。
- `attemptreopenmethod` 为 `untilpass` 71 个、`manual` 12 个（上游 4.4 起弃用
  `none`）；`maxattempts` 为 1 的 61 个、2 的 4 个、-1（不限）的 18 个。
- 文件提交 80 个：`maxfilesubmissions` 为 20 的 60 个，1、2、5 的共 20 个；
  `maxsubmissionsizebytes` 多为 250 MiB，也有 1、20、50 MiB；`filetypeslist`
  50 个为空（不限），其余如 `.pdf`、`.pdf,.docx,.doc`、`.ipynb`、`document`、`*`。
  在线文本 8 个（6 个同时有文件，未开启字数限制）。`comments` 全部开启。

可否编辑只随时间变化，与 Moodle 上游的开放规则一致；83 个作业的 `canedit` 与
`caneditowner` 都相同：

| 时间段 | 作业数（本季+上季） | `canedit` | 提交状态 |
|---|---|---|---|
| 开放前（`allowsubmissionsfromdate` 未到） | 1（1） | false | new |
| 截止前 | 2（2） | true | submitted |
| 已过截止，没有 `cutoffdate` | 46（17） | true | submitted 44、new 2 |
| 已过 `cutoffdate` | 33（12） | false | submitted 20（12 个已评分）、new 13 |
| 线下作业 | 1（0） | false | `submissionsenabled=false` |

已有提交为 1–3 个文件（PDF、Word、PowerPoint、ipynb、TeX、zip、图片、Python、
CSV），6 个文件加在线文本的提交在线文本均为空。所有已评分的提交都已过
`cutoffdate`，因此没有观察到「已评分但仍开放」的情况。

小测 5 个（1Q 3、2Q 1、3Q 1），均已关闭：`attempts=1`、`timelimit=0`、
`grademethod=1`（最高分）、`preferredbehaviour=deferredfeedback`。本季和上季的
两个各有 1 次 `finished` 尝试，访问规则只有 `quizaccess_numattempts` 和
`quizaccess_openclosedate`，没有口令和 Safe Exam Browser。关闭后
`overduehandling`、`browsersecurity` 等字段不返回；`password` 字段只返回给管理者。

### 写入实测（经用户同意）

用一个一次性的被忽略测试执行：写调用以原始表单发出，上传用手写的 multipart，因为
Core 的 HTTP 客户端只发表单和 JSON。这个测试运行后已从仓库删除：在闭合的写入枚举
实现之前，仓库里没有调用 Moodle 写函数或上传端点的代码，`cargo test -- --ignored`
不会写入任何人的 LMS。它自己挑作业：
个人作业、草稿关闭、无声明、无计时、`sendnotifications=0` 且
`sendlatenotifications=0`（不通知教师）、当前提交为 `new` 且没有文件和文本、
`canedit=true`、未评分；优先同时有在线文本的，其次截止最早的。结束时无论中途
成败都撤回，并断言提交回到 `new`、没有文件和文本。

符合条件的作业里没有启用在线文本的，所选作业只有文件提交：已过截止、没有
`cutoffdate`，`filetypeslist=.pdf,.docx,.doc`，最多 20 个文件、250 MiB，
`completionsubmit=1`。测试文件是 193 字节的空白 PDF 和 24 字节的 txt。整个过程
约 4 秒：

| 步骤 | 结果 |
|---|---|
| `upload.php`，`itemid=0`，`test-a.pdf` | HTTP 200；一条记录，键为 `author`、`component`、`contextid`、`filearea`、`filename`、`filepath`、`filesize`、`itemid`、`license`、`source`、`userid`，`itemid` 为新草稿区 |
| 同一草稿区再传 `test-a.pdf` | 一条 `errortype=filenameexist`，键为 `error`、`errortype`、`filename`、`filepath`、`size`；不覆盖 |
| `save_submission`（`files_filemanager`） | `[]`；状态 `submitted`，1 个文件，`canedit` 仍为 true |
| 新草稿区只放 `test-x.txt` 再保存 | `[]`；文件区只剩 txt。类型限制不在 WS 路径上检查，旧 PDF 被删除 |
| 新草稿区放 `test-a.pdf` 再保存 | `[]`；文件区只剩这个 PDF，`timemodified` 前进 3 秒 |
| `remove_submission` | `status=true`、`warnings=[]`；状态 `new`、`attemptnumber=0`、没有文件，`timemodified` 不变 |
| 再次 `remove_submission` | `status=false`，warning `submissionnotfoundtoremove` |

最终状态与基线相同（`new`、0 个文件、`notgraded`、`canedit=true`），只有提交记录的
`timemodified` 从原值变成最后一次保存的时间：撤回不更新它，上游也把它作为成绩簿的
`datesubmitted` 写回。状态为 `new` 时网页不显示这个时间。实测还留下了：Moodle 日志里
的上传、保存、撤回事件；`completionsubmit=1` 使完成度先变为完成、撤回后改回未完成；
站点开启 `submissionreceipts` 时，三次保存各给学生发一条回执。

### 上游行为（Moodle 4.5.10 源码）

开放规则（`assign::submissions_open`）：用户被锁定时关闭；时间窗为
`allowsubmissionsfromdate ≤ 现在 ≤ max(cutoffdate, extensionduedate)`，没有
`cutoffdate` 时开放后一直开放，`duedate` 本身不关闭提交；开启草稿且状态为
`submitted` 时关闭；成绩簿中该成绩被锁定时关闭。评分本身不关闭提交。
`canedit` 还要求 `mod/assign:submit` 权限和至少一个已启用的提交插件；团队作业开启
`preventsubmissionnotingroup` 而本人不在组里时为 false。

`mod_assign_save_submission`：

- 未开放时不保存，返回 warning `couldnotsavesubmission`，`item` 为
  「The due date for this assignment has now passed」，原因不区分开放前、
  cutoff 后或被锁定。成功时返回空数组。warning 的 `item` 是本地化的通知文本，
  `message` 是固定英文。
- 开放时先在缺少提交记录时新建一条 `new` 记录，再检查内容：所有插件都为空时返回
  `submissionempty`（「Nothing was submitted」）。状态设为 `submissiondrafts ?
  draft : submitted`；用户被锁定时抛出 `submissionslocked`。
- 每个已启用的提交插件都会执行保存，与 `plugindata` 里有没有它的键无关。
- 不检查提交声明（只有网页表单和 `submit_for_grading` 检查）；WS 参数里没有
  `lastmodified`，没有并发保护。草稿关闭时每次保存都触发
  `assessable_submitted` 事件和通知：学生回执取决于站点设置 `submissionreceipts`，
  教师通知取决于 `sendnotifications`（迟交时还有 `sendlatenotifications`）。
- 文件插件：`files_filemanager` 缺省或为 0 时不动文件区；否则用草稿区**整体替换**
  文件区，草稿区里没有的旧文件被删除（已实测）。超过 `maxsubmissionsizebytes` 或
  `maxfilesubmissions` 的文件被静默跳过，不报错；`filetypeslist` 在这条路径上不检查
  （已实测：`.txt` 存进了只收 `.pdf,.docx,.doc` 的作业），只有网页文件选择器检查；
  允许子目录。要保留已提交的文件，必须把它重新放进新的草稿区（Moodle App 也是
  下载后重新上传）。
- 在线文本插件：读取 `onlinetext_editor` 时不检查它是否存在。在同时启用文件和在线
  文本的作业上只发 `files_filemanager`，文本和格式会变成 NULL；`onlineformat` 列为
  NOT NULL，预计数据库写入异常、整次保存失败，而不是「文本保持不变」。上游默认顺序
  是在线文本、文件、评论，所以失败多半发生在替换文件之前；学校的插件顺序看不到。
  因此每次保存都要带上所有已启用插件的数据，文本不变时也要回填当前文本。

`webservice/upload.php`：同一草稿区内同名文件返回 `filenameexist`（已实测），单个
文件超过站点上限返回 `fileoversized`，两者都是 HTTP 200 中的逐文件条目；超过用户
上限时抛出 `userquotalimit`。cron 会清除 4 天前的草稿区。

`mod_assign_submit_for_grading`：不检查 `submissiondrafts`，也不检查内容是否为空。
草稿关闭时状态已是 `submitted`，调用只会失败；若状态还是 `new`，会把空提交标为
`submitted`。需要声明而 `acceptsubmissionstatement` 为 false 时返回 false。通知
数组按值传递，warning 总是 `couldnotsubmitforgrading`，不带原因。

`mod_assign_remove_submission`：没有提交或状态为 `new`、`reopened` 时返回
`submissionnotfoundtoremove`（已实测）；未开放时返回 `couldnotremovesubmission`。
成功时状态回到 `new`（已实测；第 2 次及以后的尝试回到 `reopened`），每个已启用插件
删除自己的数据：
文件被删除，在线文本记录被删除，启用「提交即完成」时完成度改回未完成，
`timemodified` 不变。学生无法恢复被删除的内容。团队作业的预检查用的是个人提交记录。

`mod_assign_start_submission`：只在站点开启 `enabletimelimit` 且作业
`timelimit>0` 时有效，否则返回 `timelimitnotenabled`；未开放时
`submissionnotopen`，已有进行中的尝试时 `opensubmissionexists`。成功后写入
`timestarted`，计时无法撤销；保存路径不因超时拒绝，只标为迟交。撤回提交不清除
`timestarted`。

新尝试只能由教师产生：`untilpass` 在评分低于成绩簿及格线且还有次数时自动重开，
`manual` 由教师重开。新尝试从空提交开始。团队作业里一人保存即为全组的提交，
`requireallteammemberssubmit=0` 时组员记录一起更新，撤回也作用于全组。

### 可撤回性判定表

「改」指再次保存（整体替换内容）；「撤回」指 `remove_submission`，会删除内容。

| 条件 | 保存后还能改 | 能撤回 | 依据 |
|---|---|---|---|
| 草稿关闭（本站全部），已开放，未到 `duedate`，未锁定，成绩簿未锁定 | 能 | 能，内容删除 | 实测 2 个 `canedit=true`；上游 |
| 同上，已过 `duedate`，未过 `cutoffdate` 或没有 `cutoffdate` | 能，标为迟交；没有 cutoff 时没有期限 | 能 | 实测 46 个（均无 cutoff），其中 1 个做过保存、改、撤回；上游 |
| 已过 `cutoffdate`（有延期时取较晚者） | 不能，最终 | 不能 | 实测 33 个 `canedit=false`；上游 |
| 开放前 | 不能保存 | 无内容可撤回 | 实测 1 个；上游 |
| 已评分，仍在开放期 | 上游能；本站未观察到 | 上游能 | 待确认 |
| 教师锁定（`locked`）或成绩簿锁定 | 不能，最终 | 不能 | 上游；本站未观察到 |
| 草稿开启（本站未见）：状态为 `draft` | 能 | 能 | 上游 |
| 草稿开启：`submit_for_grading` 之后 | 不能，除非教师退回草稿 | 不能 | 上游 |
| 需要提交声明（本站未见） | 草稿开启时提交需 `acceptsubmissionstatement=true`；草稿关闭时 WS 保存不检查声明 | 按开放规则 | 上游；Core 仍应先让用户确认声明 |
| 计时作业（本站未见） | `start_submission` 后计时不可撤销；之后按开放规则可改，超时只标迟交 | 能，但 `timestarted` 保留，计时继续 | 上游 |
| 团队作业（本站未见） | 一人保存即全组提交；`preventsubmissionnotingroup` 且不在组里时不能改 | 能，作用于全组 | 上游 |
| 教师重开的新尝试 | 新尝试从空开始，旧尝试只读 | 撤回后回到 `reopened` | 上游 |

### 写入枚举候选

本站作业都是草稿关闭的文件或在线文本提交，最小的闭合集合是三项：

1. 上传草稿文件：`POST webservice/upload.php`，multipart，`token` 字段，第一个文件
   用 `itemid=0` 并取回 `itemid`，其余文件用同一 `itemid`，`filepath=/`。逐文件检查
   `errortype`。
2. 保存提交：`mod_assign_save_submission`，`files_filemanager` 为上面的 `itemid`，
   每个已启用插件都要带数据（在线文本不变时回填当前文本）。草稿区必须是最终的完整
   文件集合，包括要保留的旧文件。
3. 撤回提交：`mod_assign_remove_submission`（本人 `userid` 来自 site info），
   会删除内容，只在用户明确确认后执行。

按需再加：`mod_assign_submit_for_grading` 只在作业 `submissiondrafts=1` 且状态为
`draft` 时可用，声明由用户本人确认；`mod_assign_start_submission` 只用于计时作业，
因计时不可撤销，需要单独确认。`core_files_get_unused_draft_itemid` 和
`core_files_delete_draft_files` 不需要：每次用新的草稿区，旧草稿由 cron 清除。

类型限制要由 Core 在上传前检查：学校不检查。执行后用
`mod_assign_get_submission_status` 核对：状态、文件名、文件大小和数量（数量和大小
可能被静默截掉）、在线文本、`timemodified` 是否前进。写函数返回非空
`warnings` 即为失败；空数组也不能单独证明成功。`SubmissionCapabilities` 可由
`lastattempt.canedit`、`submissiondrafts`、`requiresubmissionstatement` 和声明文本、
各插件的数量、大小、类型限制、开放/截止/cutoff 时间、`timelimit`、团队字段、
`locked` 和 `graded` 推出。

### 只能通过写入回答的问题

写入实测已回答：`upload.php` 的返回字段和同名处理；类型限制不检查；重新保存整体
替换文件区并推进 `timemodified`；撤回后回到 `new`、不改 `timemodified`，已过截止而
没有 cutoff 时也能撤回。仍待回答：

1. `upload.php` 的网关大小上限和病毒扫描。
2. 同时启用文件和在线文本时只发 `files_filemanager` 的结果（上游推断：数据库异常、
   整次失败），以及只发 `onlinetext_editor` 时文件是否保留（上游：保留）。没有空白的
   文件加在线文本作业可测。
3. 超出数量或大小限制的文件是静默丢弃还是整次拒绝（上游：静默丢弃）。所测作业限制
   为 20 个、250 MiB，没有测。
4. 学生回执是否真的发出（取决于站点设置）。
5. 已评分但仍开放时能否修改，修改后 `gradingstatus` 如何变化。
6. 学校是否改动过 `mod_assign`；目前只能确认版本号，所测路径与上游一致。
7. `mod_ttws_*` 的参数和用途，是否就是课程评价。
8. 草稿区 4 天清理和 `userquota` 是否计入草稿区。
