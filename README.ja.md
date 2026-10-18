# IsctCompanion

**東京科学大学（旧東京工業大学）の学生向けデスクトップアプリ。**
T2SCHOLA の授業・時間割・課題と、配信された試験予定を macOS / Windows のひとつのワークスペースにまとめます。

[English](README.md) · **日本語**

[最新版をダウンロード](https://github.com/isct-companion/isct-companion/releases/latest) · [不具合を報告](https://github.com/isct-companion/isct-companion/issues) · [開発への参加](CONTRIBUTING.md) · [ドキュメント](docs/README.md)

## 主な機能

- **Today：** 今日の授業、関連付けた今後の試験、課題の締切と完了済み課題を確認。
- **Lecture Desk：** クォーターごとに授業を探し、LMS 教材・課題・ローカルノートを開く。
- **時間割：** 過去のクォーターを含む LMS 時間割と個人の予定を表示。
- **試験：** 授業と一致する試験をクォーター別に表示。Filter では公開試験を検索し、右側で授業の関連付け・日時・教室を編集。
- **通知：** 授業・試験・課題ごとに通知と事前通知の時間を設定。
- **学内サイト：** T2SCHOLA と教務 Web をアプリ内で利用。任意で自分のクラウドアカウントへバックアップ。

![Lecture Desk で試験を検索・編集](docs/screenshots/exams.png)

![試験通知の独立した設定](docs/screenshots/exam-reminders.png)

## インストール

[Releases](https://github.com/isct-companion/isct-companion/releases/latest) から対応するインストーラーを選んでください。

| 環境 | ファイル名の末尾 |
| --- | --- |
| macOS 14 以降・Apple Silicon | `aarch64-apple-darwin.dmg` |
| macOS 14 以降・Intel | `x86_64-apple-darwin.dmg` |
| Windows 10/11・x64 | `x86_64-pc-windows-msvc.exe` |

macOS のリリース版は署名・公証済みです。Windows 版は現在未署名です。
モバイル版の画面は開発中です。

初回起動後、**Settings → School accounts** で学内アカウントを接続し、データを更新します。
**Lectures → Exams** でクォーター別の試験を確認できます。授業と一致する試験、または自分で関連付けた試験が個人の予定に表示されます。

## データの取得元

| 情報 | 取得元 |
| --- | --- |
| 授業・時間割・教材・課題・提出状況 | T2SCHOLA / LMS |
| 授業検索・履修登録情報・正式な成績 | 教務 Web |
| 授業の説明・シラバス | 東京科学大学シラバス |
| 試験・学事参考日程 | 設定された参照データの購読元 |
| ノート・個人予定・個人修正 | ローカルワークスペース |

個人修正は取得元の更新後も保持されます。ワークスペースはローカルの SQLite に、認証情報は OS の資格情報保管庫に保存されます。
詳しくは [データの所有関係](docs/data-ownership.md) を参照してください。

## 開発・参加

[CONTRIBUTING.md](CONTRIBUTING.md)、[開発ガイド](docs/development.md)、[テストガイド](apps/desktop/tests/README.md) から始めてください。
[ドキュメント一覧](docs/README.md) に Core API、ネイティブ SDK、取得処理、配布手順をまとめています。
レビュー済みのバージョンは、一致するバージョンタグの push で配布できます。[リリース手順](docs/desktop-release.md) を参照してください。

IsctCompanion は独立したプロジェクトです。利用・配布には [日本語利用規約](docs/legal/EULA.ja.md) と [English EULA](docs/legal/EULA.md) が適用されます。
