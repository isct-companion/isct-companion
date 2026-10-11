# 设置精简方案

## 原则

参考 iOS 设置、Things 3、Linear、GNOME HIG 的共同做法：

1. **默认值替代设置**：选项能定一个好默认值的，就不让用户来选。GNOME HIG 把设置项视为最后手段。
2. **设置只放偶尔要改的**：日常操作（刷新、恢复隐藏项）放回它发生的地方，不进设置页。
3. **不解释显而易见的**：标题能说清楚的不加副标题。说明文字只留给三种情况：会删数据、涉及隐私、结果不直观。iOS 设置的分组脚注只在这三种情况下出现。
4. **一个概念只出现一次**：同一件事不分散在两个页面。
5. **渐进披露**：给开发者用的东西放在一个开关后面，普通用户看不到。

用户心智模型：ISCT 学生打开设置，只是为了「登账号、选系和入学年、开通知、换外观、备份」。其他东西都应该自动完成。

## 现状

一共 14 个分区，基础层就有 8 个，每页都有一句副标题。

| 分区 | 设置项 | 现有说明文字 | 问题 |
|---|---|---|---|
| School accounts | Science Tokyo / Tokyo Tech 账号、矩阵卡 | — | 保留 |
| | Refresh school records | Reads your courses, assignments and grades again now | 日常操作，应在 Today |
| | Reopen the last school page | Web opens where you left off, even after a restart | 应作为默认行为 |
| Study profile | Department、Year you started | Used to suggest courses…／The course finder filters…／Your study guide follows this year… | 三句解释都多余 |
| | 学修案内状态、Check now | {{year}} study guide · {{count}} listed courses… | 数据状态，不是设置 |
| Notifications | Allow notifications、Class reminders、Remind me、Assignment reminders | Reminders for classes and deadlines, shown while the app is open. 等 | 「已允许」状态行多余 |
| Academic calendar | 节假日/调课、Quarters、Important dates、导入/导出 | Holidays, make-up days and quarter dates. Official dates update by themselves; your own changes always win. | 官方数据会自动更新，绝大多数人用不到；导入导出和备份重复 |
| Appearance & language | Language、School content language、Theme、Text size、Density、Background image、Keyboard shortcuts | How the app looks on this device. 等 | 快捷键是说明不是设置；背景图要粘链接，太偏 |
| Course files（桌面端） | Course folder、自动下载大小、清理天数、Show one language in bilingual LMS | Where course materials are saved… 等 | 双语显示和「内容语言」是同一件事 |
| Backup & sync | Cloud service 下拉、Connect、Restore、Back up now、文件导出/导入、Erase | Save your plans, calendar changes and preferences… | 只剩 Google Drive 一个选项，下拉框没意义 |
| Hidden assignments | 恢复列表 | Assignments you hid from Today. Hiding never changes anything on the LMS. | 应在 Today 里恢复 |
| Data updates（高级） | 各数据源状态、Check now、Update on mobile data | Calendars, exam timetables and study guides come from a public, reviewed data set… | 应在后台自动完成，用户不用关心 |
| Automation（高级） | 定时任务列表、Run now | Jobs the app runs on its own while it is open… | 开发者才会看 |
| Developer options（高级） | Developer 开关、Academic profile JSON、Data source URLs、OAuth client ID | Tools for people who write code… | Client ID 已在构建时确定，可删 |
| AI agent access / Site styles & scripts / Diagnostics | — | 各一句副标题 | 留在开发者层即可 |

## 精简后

基础层从 8 个分区减到 5 个，每页不再有副标题。

### 1. 账号 Accounts
- Science Tokyo 账号
- Tokyo Tech 账号（矩阵卡）

删除：Refresh school records（改到 Today，下拉刷新或按钮）；Reopen the last school page（改为默认行为）。

### 2. 学业 Study
- Department
- Year you started
- Academic calendar →（二级页面，不在侧栏）

删除：三句解释、学修案内状态行；日历的导入/导出（由备份覆盖）。

学年历编辑方式：整行可点，编辑框在该行下方展开（删除/恢复官方也在编辑框里）；去掉每行的 Official 标签和 Change 链接，只有自己改过或添加的日子在副标题里注明；过去的日子默认折叠；状态栏只在出问题时出现。

### 3. 通知 Notifications
- Class reminders：开关 + `10 min before` 选择
- Assignment reminders：开关

权限只在被系统拒绝时显示一行：`Notifications are blocked in System Settings.`

### 4. 外观 Appearance
- Language
- School content language（同时决定双语 LMS 只显示哪种语言，合并原 Files 里的选项）
- Theme
- Text size

- Show one language in bilingual LMS（从 Files 移来）

删除：Keyboard shortcuts 说明行；Density、Background image 收进折叠的 More options。

### 5. 备份 Backup
- Google Drive：`Connect` / 已连接后 `Back up now`、`Restore`
- Backup file：`Export…`、`Import…`
- Erase this device…（放最底部，红色）

删除：Cloud service 下拉框和 Off 选项；分区副标题。

### 桌面端额外：文件 Files
- Course folder
- Download new files automatically（保留大小上限）

删除：清理天数（用默认值）；双语选项（移到外观）。

### 移出设置
| 原位置 | 新位置 |
|---|---|
| Hidden assignments | Lectures → Assignments 列表底部，折叠的「Hidden assignments」，每行可 Restore |
| Refresh school records | 删除；学校数据过期会自动拉取，Schedule 里仍可手动拉 |
| Reopen the last school page | 改为默认行为 |
| Data updates、Automation | 高级层 |
| Density、Background image | 外观页的 More options（折叠） |
| OAuth client ID | 删除 |

### 高级 Advanced（有一定能力、但不碰代码的人）
Data updates、Automation、AI agent access（MCP）、Developer options（开关）。侧栏 Advanced 标题左移、加分隔线。

### 开发者层（Developer options 打开后才出现，纯 CS 用）
Site styles & scripts（CSS/JS）、Diagnostics；Developer options 页内的 Academic profile JSON、Data source URLs。

### 关于 About（侧栏最底部，分隔线下）
版本号、源代码链接、主要开源组件及其许可证。

外观里的 Density、Background image 收进外观页底部默认折叠的 More options。

## 文案规则与改写

规则：标题用名词或动词短语；不写「This lets you…」「Used to…」；状态用一个词；确认框只说后果。

| 现在 | 改为 |
|---|---|
| Save your plans, calendar changes and preferences, and bring them to another device. School data is downloaded again from the school. | （删除） |
| How the app looks on this device. | （删除） |
| Used to suggest courses and to pick your study guide. | （删除） |
| Your study guide follows this year and stays the same as you move up | （删除） |
| Course names, rooms and syllabus text. Automatic uses the menu language. | （删除，选项名 Automatic 已足够） |
| Notifications are allowed on this device. | （删除，只在被拒绝时提示） |
| Reminders for classes and deadlines, shown while the app is open. | Only while the app is open.（如确有此限制，这是不直观的结果，保留一句） |
| One day and one hour before the deadline, until you submit | 1 day and 1 hour before |
| Connect {{service}} to start backing up. | （删除，按钮本身就是 Connect） |
| Cloud backup is off. | （删除） |
| Removes plans, calendar changes and preferences, then starts empty | Can't be undone.（会删数据，保留一句后果） |
| Replace the data on this device with the cloud backup? | Replace this device's data with the cloud backup? |
| Assignments you hid from Today. Hiding never changes anything on the LMS. | （随页面移走；如保留，只留后半句：Hiding doesn't change the LMS.） |

## 结果

| | 现在 | 精简后 |
|---|---|---|
| 基础分区 | 8 | 5（桌面 6） |
| 基础设置项 | ~30 | ~16 |
| 页面副标题 | 每页一句 | 0 |
