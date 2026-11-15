# ScienceTokyoExam の API 変更点と、連携にあたってのお願い（IsctCompanion）

日付：2026-11-15
比較：計画書の基準 `39c1563`（2026-11-12）→ 現在の `main`（2026-11-15、PR #245 を含む）

計画書（v3.1）と依頼文の API 一覧は、11/12 の時点のものです。その後、講義マスタを作り直して API を整理したので、**講義まわりの API はほぼ全部が入れ替わり**、資料とリクエストの形も変わりました。さらにアプリへ渡す前に、エラーコードと通報の形を整え、`GET /api/client-config` を足しました（PR #245）。1〜6 節では、変わった点を、アプリが使う部分に絞ってまとめます。7 節は、IsctCompanion を当サイトの公式のネイティブアプリとして出すにあたってのお願いと確認、8 節はこちらで済ませた設定です。

---

## 1. いちばん大きな変化：講義は「科目コード・時間割番号・教員 ID」で結ぶ

- 講義の ID は**大学の科目コード**です（以前の `courseId` は廃止）。
- クラスは**時間割番号**（`classId`、数値）で表します。
- 教員は `teachers: [{ id, name }]` で表し、**ID で結びます**。教員名だけを書く自由入力の欄はなくなりました（一覧にいない教員を足すときは 3.4 の `newTeachers`）。
- 資料は、科目（複数可）と教員（0 人以上）の ID だけで講義に結び付きます。名前では照合しません。

**アプリへの影響**：時間割の科目コードで `GET /api/subjects/:code/view` を**直接引けます**。そのため、計画書 §5.4 の次のものは要らなくなります。名前で候補を探すこと、利用者に選んでもらう流れ、選んだ結果の記憶、T6（科目コードで講義を引く API）。

講義ビューが 404 `course.not_found` を返したときは、そのコードで `GET /api/subjects/search?q=<コード>` を引いてください。合同開講の別コードなどがこれに当たり、検索は別コードも対象にしているので、正式の科目コードが分かります。リクエストと投稿には、正式の科目コードを使ってください。

## 2. 消えた API と代わり

| 計画書にある API | 今 | 代わり |
|---|---|---|
| `GET /api/search` | 廃止 | `GET /api/subjects/search?q=&limit=` |
| `GET /api/courses/suggest` | 廃止 | `GET /api/subjects/search` |
| `GET /api/courses/search-suggest` | 廃止 | `GET /api/subjects/search` |
| `GET /api/courses/:courseId/view` | 廃止 | `GET /api/subjects/:code/view`（`:code` は科目コード） |
| `GET /api/exams/by-storage-path` | 廃止 | 不要（409 `already_registered` が `examId` を返す。3.4） |
| `CourseCatalog.byCourseCode`（T6 の材料） | 廃止 | 不要（`/api/subjects/:code/view` を科目コードで直接引ける） |

## 3. 形が変わった API

### 3.1 科目の検索 `GET /api/subjects/search`

- 入力：`q`（100 文字まで）、`limit`（既定 20、最大 50）
- 出力：`{ subjects: SubjectHit[] }`

```ts
SubjectHit = {
  code: string; name: string; nameEn: string | null;   // 科目
  examCount: number;
  rows: Array<{                                         // 最新年度のクラスを「同じ教員の並び・同じ開講時期」でまとめた行
    classIds: number[]; labels: string[]; taughtIn: string;
    teachers: { id: string; name: string }[];
    examCount: number;
  }>;
}
```

### 3.2 講義ビュー `GET /api/subjects/:code/view`（ETag あり）

```ts
SubjectView = {
  subject: { code; name; nameEn };
  latestYear: number | null;
  rows: ClassRow[];                 // 上の rows と同じ形（examCount なし）
  currentTeachers: Teacher[]; pastTeachers: Teacher[];
  exams: Exam[];                    // 公開中の資料。年度・回の並べ方はクライアントが決める
  openRequests: Array<{
    id: string; academicYear: number | null;
    occasion: ExamOccasion | null; wantedRole: 'question' | 'answer' | null;
    teacherIds: string[];
  }>;
}
```

- 古い講義 ID で呼ぶと `{ movedTo: string | null }` が返ります（`null` は非表示にした講義）。
- 以前のように年度ごとの行にまとめた形ではなく、**`Exam` をそのまま並べて返します**。

### 3.3 資料 `Exam`（`/api/exams/:id`、`by-ids`、`/api/me/exams`、講義ビューで共通）

`title`、`type`、`year`、`kind`、`courseName`、`instructor` はなくなりました。管理用の `reportsCount`、`needsReview`、`fileHash` も、一般の API では返しません。今の形は次のとおりです。

```ts
Exam = {
  id: string;
  subjects: { code; name; nameEn }[];
  teachers: { id; name }[];               // 0 人なら「教員不明」
  academicYear: number | null;
  occasion: ExamOccasion | null; occasionNumber: number; variant: string;
  role: ExamDocumentRole | null; provenance: ExamProvenance | null; answerSource: AnswerSource | null;
  note?: string;
  storagePath: string; fileName: string; fileType: string; fileSize: number;
  authorId: string;                       // 自分の資料かどうかは、これと自分の uid を比べて判定する
  pairKey: string | null;                 // 問題と解答の組のキー
  createdAt: number; updatedAt: number;   // epoch ミリ秒
}
```

決まった値の一覧：
- `occasion`：`midterm` `final` `makeup` `quiz` `check` `report` `exercise` `textbook` `mock` `none` `unknown`
- `role`：`question` `answer` `both` `notes` `other`
- `provenance`：`original` `reconstructed`
- `answerSource`：`official` `student` `unknown`

### 3.4 投稿 `POST /api/exams`

本文：

```ts
{
  subjectCodes: string[];            // 1 件以上、10 件まで
  teacherIds: string[];              // 40 人まで。講義ビューや検索で返る教員 ID
  newTeachers: { familyName; givenName }[];   // 一覧にいない教員を足すときだけ。familyName は必須、givenName は空でもよい
  academicYear: number | null;       // 1950〜翌年
  occasion; occasionNumber;          // occasionNumber は 0〜30。quiz/check/report 以外は 0
  variant: string; role; provenance;
  answerSource: AnswerSource | null; // role が answer か both のときは必須
  note: string | null;               // 40 文字まで
  storagePath; fileHash: string | null; fileName; fileType; fileSize;
  requestId: string | null;          // 応えるリクエスト
}
```

- `role` が `notes` か `other` のときは `occasion: 'none'`、それ以外のときは `none` 以外の値にしてください。
- 教員は、講義ビューが返す `teacherIds` から選ばせてください。`newTeachers` は、本当に一覧にいない教員のときだけ使ってください。同じ表記の教員がすでにいればその教員が使われ、いなければ教員の表に行ができます（表記の揺れで、同じ教員の行が重複してできるのを防ぐため）。
- **題名（`title`）の欄はなくなりました。** テスト投稿の印（T7）は `note` に入れてください。
- `requestId` を付けた投稿でも、リクエストが「応えた」になるのは、**資料の科目にリクエストの科目が含まれるときだけ**です。含まれなければ、資料だけが作られます。
- 応答：
  - 成功は **201** `{ ok: true, examId }`
  - 同じ `storagePath` の資料がすでにあるときは、**409 `upload.already_registered`。`params.examId` に既存の資料の ID が入ります**。結果が分からないまま再送したときは、これで完了とみなせます（`storagePath` は自分の名前空間のものなので、この ID は自分の資料です。その資料を自分で削除していた場合は、`GET /api/exams/:id` が `{ exam: null }` を返します）。
  - 投稿の上限に達したときは **429**。`upload.quota_daily` または `upload.quota_total` で、`params.limit` に上限の件数が入ります。
- 受け付けるファイルの形式は `application/pdf`、`image/jpeg`、`image/png`、`image/webp`、`text/plain` です。

### 3.5 リクエスト

- `POST /api/requests` の本文は `{ subjectCode, classId: number | null, academicYear: number | null, occasion: ExamOccasion | null, wantedRole: 'question' | 'answer' | null, description }` です（`title`、`courseName`、`instructor` はなくなりました）。
  - `occasion` に `none` と `unknown` は使えません。回が分からないときは `null` を送ってください。
  - 成功は **200** `{ id }` です（201 ではありません）。1 日 20 件を超えると 429 `request.daily_limit` です。
- `GET /api/requests` は**募集中だけ**を返します（`tab` と `mine` はなくなりました）。入力は `cursorCreatedAt` と `cursorId`、出力は `{ items, cursor }` です。自分の解決済みのリクエストは `GET /api/me/requests` で取ってください。
- `RequestModel` の形：`{ id, subject, classId, teachers, academicYear, occasion, wantedRole, description, authorId, status: 'open' | 'fulfilled' | 'closed', createdAt, updatedAt, answerExamId? }`

### 3.6 通報 `POST /api/exams/:examId/report`

- 本文は `{ reason, detail }` です。
  - `reason`：`copyright`（著作権侵害）`inappropriate`（不適切なコンテンツ）`wrong_answer`（解答の間違い・不完全）`broken_file`（ファイル破損・読み取り不可）`other`（その他）
  - `detail`：補足。900 文字まで、空でも構いません。
- 以前の「日本語の理由: 補足」を 1 つの文字列で送る形はなくなりました。表示の文言はアプリ側で自由に決めてください。
- 同じ資料を 2 回通報すると、409 `exam.report_duplicate` です。

### 3.7 新規：`GET /api/client-config`（ログイン不要）

- 応答は `{ minAppVersion: string | null }` です（`x.y.z` の形、未設定なら `null`）。5 分キャッシュします。
- アプリの版がこれより古ければ、機能を隠して更新を促してください。

## 4. エラー

- どの API も、エラーの本文は `{ error: string, code: string, params?: object }` です。`code` は必ず入ります。`error` は予備の日本語の文で、表示はアプリが `code` から決めてください。
- 計画書 §2 の「`register` と `claims/refresh` は `{error}` だけで `code` がない」は、もう当てはまりません。この 2 本も `code` で扱ってください。`register` の成功時の応答は `{ ok: true }` です。
- 詳しい形は `src/shared/api/contracts/` が、エラーコードの全体と文言は `src/shared/i18n/messages/ja.server.ts` が正本です。主なコードは次のとおりです。

| 状況 | HTTP | code |
|---|---|---|
| トークンがない・無効 | 401 | `auth.login_required` |
| 許可されていないアカウント | 403 | `auth.not_allowed` |
| 未登録（退会済み） | 403 | `auth.not_registered` |
| 短い時間にリクエストが多すぎる（uid ごとに 15 分で 300 回。トークンが無ければ IP ごと） | 429 | `rate.too_many` |
| 投稿の上限 | 429 | `upload.quota_daily`、`upload.quota_total` |
| 投稿の入力 | 400 | `upload.subject_required`、`upload.teacher_invalid`、`upload.role_invalid`、`upload.occasion_invalid`、`upload.occasion_number_invalid`、`upload.provenance_invalid`、`upload.answer_source_invalid`、`upload.year_invalid`、`upload.note_too_long`、`upload.subject_or_teacher_not_found` |
| 投稿のファイルの指定 | 400 | `upload.type_not_allowed`、`upload.too_large`、`upload.format_mismatch`、`upload.file_empty` |
| 二重登録・重複・ファイルが無い | 409 | `upload.already_registered`、`upload.duplicate_file`、`upload.file_missing` |
| 置かれたファイルが大きすぎる | 413 | `upload.stored_too_large` |
| リクエストの上限 | 429 | `request.daily_limit` |
| 講義が無い | 404 | `course.not_found` |
| 巻き戻しの対象が登録済み | 409 | `upload.rollback_in_use` |
| ファイルが無い | 404 | `exam.file_not_found` |
| 通報の理由が不正 | 400 | `exam.report_reason_required` |
| 書き込みの一時停止 | 503 | `write_freeze` |
| サーバーの準備中 | 503 | `server.not_ready` |
| 存在しない API | 404 | `server.not_found` |

- `rate.too_many` と `write_freeze` と `server.not_ready` は一時的なものです。間隔を空けて再試行してください。

## 5. アプリが使う API の一覧（T4 で形を保つ対象）

| 用途 | API |
|---|---|
| 設定 | `GET /api/client-config` |
| アカウント | `POST /api/auth/claims/refresh`、`POST /api/auth/register`、`GET /api/me`、`POST /api/users/delete`（退会） |
| 検索・講義 | `GET /api/subjects/search`、`GET /api/subjects/:code/view` |
| 資料 | `GET /api/exams/:examId`、`POST /api/exams/by-ids`、`POST /api/files/read-url`、`POST /api/me/views/:examId` |
| 投稿 | `POST /api/exams/hash-exists`、`POST /api/exams/uploads/authorize`、`POST /api/exams`、`POST /api/exams/uploads/rollback`、`POST /api/exams/:examId/delete`、`GET /api/me/exams` |
| 保存 | `GET /api/me/saved-exams`、`POST /api/me/saved-exams/:examId` |
| リクエスト | `GET /api/requests`、`GET /api/me/requests`、`POST /api/requests`、`POST /api/requests/:requestId/status`、`POST /api/requests/:requestId/delete` |
| 通報 | `POST /api/exams/:examId/report` |

退会は、App Store の規定に合わせて、アプリの中から `POST /api/users/delete` を呼んで構いません。退会すると、その人の投稿はすべて削除の扱いになり、本人は元に戻せないので、確認の画面を出してください。成功したら、端末に残したセッションと写しを消してください。同じ呼び出しはやり直しても安全です。

このほかの API（`/api/me/home` などの画面用の API、管理用の API）は、アプリから呼ばないでください。

## 6. 計画書で直していただきたい箇所

- **§2 の API の表と、依頼文 T4 の一覧**：上の 5 節に差し替えてください。T4 の `client-config` は、こちらで用意しました（3.7）。
- **§5.4 の講義の照合と T6**：1 節のとおり不要です。
- **§5.6 の投稿の回復**：409 `already_registered` の `params.examId` で完了とみなしてください（`by-storage-path` は使いません）。
- **§5.7 と §2 の通報**：日本語の文字列ではなく、3.6 のコードで送ってください。
- **§1 の範囲（退会を作らない）**：App Store の規定があるので、退会をアプリに入れてください（5 節）。
- **T7 のテスト投稿の印**：`note` に入れてください。
- **規約・プライバシーポリシー**：英語版ができました。アプリ内で原文を見せるときは、表示言語に合わせて日本語版か英語版を出してください。

## 7. 公式アプリとして出すにあたってのお願い

IsctCompanion は当サイトの公式のネイティブアプリになります。規約とプライバシーポリシーでアプリの扱いを説明するのはこちら（運営者）なので、次の点を教えてください。

### 7.1 データの扱いと連絡先

- **データの流れの一覧**：当サイトから得たデータ（資料のファイル、資料やリクエストの情報、アカウントの情報）について、次の 3 点を一覧にしてください。端末のどこに保存するか。どこへ送るか（クラッシュ報告や利用状況の解析の SDK を含む）。どのくらいの期間残すか。当サイトのプライバシーポリシーの記述は、この一覧をもとに書きます。
- **OAuth client と同意画面**：サインインに使う client は、そちらの GCP プロジェクトにあります。次の 2 点を教えてください。
  - そのプロジェクトの持ち主と、連絡の窓口
  - Google の同意画面に出るアプリ名と発行元

### 7.2 設計で入れていただきたいこと

- **強制アップデートを初版から入れてください**（3.7 の `client-config`）。5 節の API の形を変えるときは、こちらが古い形を 4 週間残します。その間にアプリの更新を出していただき、こちらが `minAppVersion` を上げてから、古い形を消します。
- **オフラインの写しに期限を付けてください。** 開くたびに `GET /api/exams/:id` で確かめるだけでは、オフラインのままの端末に、管理者が削除した資料が残り続けます。一定の期間（たとえば 30 日）開かれなかった写しは消してください。
- **教員は ID で選ばせてください**（3.4）。名前で照合する経路は作らないでください。

### 7.3 運用

- **リクエスト数の見積もり**：想定する利用者の数と、1 人あたりのリクエスト数（起動時と、講義を開いたとき）を教えてください。Vercel の防火壁の設定（T3）は、この見積もりをもとに決めます。なお、`X-Client` を見て上限を緩めることはしません。上限に掛かったら、間隔を空けて再試行してください。
- **本番でのテスト（T7）**：テストには、テスト専用のアカウントを使ってください。テストが終わったら、テスト投稿の ID を教えてください。こちらで purge します。

## 8. こちらで済ませたこと（T1〜T3）

- **T1**：Google プロバイダの safelist に、デスクトップ用と iOS 用の 2 つの client ID を足しました。
- **T2**：Web 用の API キー（`config/firebase/firebase-applet-config.json`）には、アプリケーションの制限がありません。Identity Toolkit と Secure Token も呼べるので、アプリでも同じキーを使えます。
- **T3**：防火壁の規則は、`/api/` に掛かる IP ごとの上限（10 秒で 60 回、超えると拒否）だけで、チャレンジを掛ける規則はありません。

計画書の第 0 歩（3 つの端末でサインインから `/api/me` まで）を進めてください。
