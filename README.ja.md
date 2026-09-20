[English](README.md) | 日本語

<p align="center">
  <img src="docs/images/agent-tutor-logo.png" alt="agent-tutor" width="720">
</p>

# agent-tutor

> 英語版が正本 ([README.md](README.md))

`agent-tutor` (`tutor`) は、人間によるフィードバックの繰り返しを通じて、Claude Code などの AI コーディングエージェントを教育・育成するためのツールです。

同じ指摘を何度も繰り返したり手動でルールを管理する代わりに、`tutor` は会話ログ（JSONL）から指定した日時の人間の発言（指摘・指示）を自動抽出します。各ターンは、直前のアシスタントの応答、実行されたツール履歴、そしてその指摘がすでにメモリや `CLAUDE.md` に反映済みかどうかのフラグとともにコンテキスト豊富に出力されます。CLI、MCP、またはインタラクティブな Web レビュー UI を通じてフィードバックをトリアージし、コーディング規約、ドキュメント、メモリ、スキル、Issue トラッカーなどへ計画的に反映案を適用できます。

## 使い方 (Usage)

```console
# 基本的な抽出
tutor                                            # 今日の人間の発言を抽出 (JSONL)
tutor --date 2026-09-17 --format md              # 特定日の発言を Markdown 形式で出力
tutor projects                                   # 未判定ターンが存在するプロジェクト一覧を表示

# インタラクティブな Web レビュー UI
tutor ui                                         # ブラウザで Web レビュー UI を起動
tutor ui --project my-project                    # 特定プロジェクトを対象にレビュー UI を起動

# 宛先設定と判定台帳
tutor destinations                               # 自動検出された反映先一覧を確認
tutor destinations --reset                       # 環境を再スキャンして宛先設定を再生成
tutor ledger                                     # 判定済み台帳を確認
tutor mark --session <SESSION_ID> --turn 1 --status approved --destination project_claude

# MCP 設定とサーバー起動
tutor install-mcp                                # ~/.claude.json に MCP サーバー設定を自動登録
tutor mcp                                        # MCP stdio サーバーとして起動
```

デフォルトでは、`--date` はローカルタイムゾーンの今日、`--projects-dir` は `~/.claude/projects`、出力形式は JSONL です。台帳（デフォルトでは `~/.local/state/tutor/ledger.json`）に `approved` または `rejected` として記録されたレビュー済みターンは、次回以降の抽出から自動的に除外されます。未判定ターンをすべて確認したい場合は `--no-ledger` を指定してください。全オプションは `tutor --help` で確認できます。

## インストールと開発 (Installation)

```console
# cargo でソースからインストール
cargo install --git https://github.com/syarihu/agent-tutor

# または Makefile でローカルビルド
make install      # リリースバイナリをビルドして ~/.cargo/bin/tutor にインストール
make dev          # デバッグバイナリをビルドして ~/.cargo/bin/tutor にシンボリックリンク
make install-mcp  # ~/.claude.json に tutor MCP サーバーを自動登録
make status       # 現在有効な tutor バイナリのパスとバージョンを表示
make test         # テストを実行
```

`make dev` モードでは、`~/.cargo/bin/tutor` が `target/debug/tutor` へのシンボリックリンクになります。コード編集後に `cargo build` を実行するだけで、再インストールなしに CLI や MCP コマンドへ変更が即座に反映されます。

### Model Context Protocol (MCP) の設定

`tutor install-mcp`（または `make install-mcp`）を実行すると、`~/.claude.json` に tutor MCP サーバーが自動登録されます：

```console
tutor install-mcp
```

手動で MCP クライアントに設定する場合は以下の通りです：

```json
{
  "mcpServers": {
    "tutor": {
      "type": "stdio",
      "command": "tutor",
      "args": ["mcp"]
    }
  }
}
```

利用可能な MCP ツール：
- `tutor_projects`: 会話ログが存在するプロジェクト一覧と未レビュー件数を取得
- `tutor_extract`: 指定日の未判定ターンと直前文脈を抽出
- `tutor_review_ui`: ブラウザで Web レビュー UI を起動し、確定結果を待機・返却
- `tutor_mark`: 判定結果（approved, rejected, deferred）を台帳に記録
- `tutor_destinations`: 利用可能な宛先テーブルと判断軸ガイドラインを取得
- `tutor_ledger`: 現在の台帳エントリーを確認

利用可能な MCP プロンプト：
- `tutor_review`: 指摘回収からトリアージ、Web レビュー UI、自動反映までの一連のワークフローを実行

## Web レビュー UI

`tutor ui`（または MCP ツール `tutor_review_ui`）を実行すると、軽量なローカル HTTP サーバーが立ち上がり、ブラウザ上で GitHub Dark 風のレビュー画面が開きます：

- **指摘のトリアージ**: 直前のアシスタントの返答やコンテキストを並べて見ながら各ターンを精査。
- **差分ドラフトと起票テンプレート**: 生成された diff や Issue の本文をインラインで直接プレビュー・編集可能。
- **柔軟な宛先選択**: `CLAUDE.md`、ドキュメント、`memory/`、スキル、外部タスク管理ツールなどへ動的にルーティング。
- **理由・修正指示入力**: 各判定ボタンの直下にあるコメント欄（`user_comment`）から、却下理由やエージェントへの追加修正指示を入力可能。
- **台帳への自動反映**: ブラウザで確定（Submit）すると、`~/.local/state/tutor/ledger.json` に判定が自動保存され、次回以降の重複レビューを防止。

## 宛先の自動検出 (Adaptive Destinations)

`tutor` はローカルマシンの環境に合わせて利用可能な反映先を自動検出します：
- **基本の規約レイヤー**: リポジトリ単位（`<cwd>/CLAUDE.md`、`<cwd>/AGENTS.md`）、グローバル単位（`~/.claude/CLAUDE.md`、`~/.config/rules/AGENTS.md`）、詳細ドキュメント（`docs/`）、一時メモリ（`memory/`）。
- **CLI & MCP 連携**: `gh`（GitHub CLI）や `linear`、`jira`、`asana`、`notion`、`lk` などのツールが PATH に存在するか `~/.claude.json` に登録されている場合、タスク起票やナレッジ登録の宛先が自動で有効化されます。

`tutor destinations` で検出結果を確認でき、`~/.config/tutor/destinations.json` を編集してカスタマイズも可能です。環境を変更した後は `--reset` を渡すことで再スキャンと再生成が行えます。

## 出力フォーマット (Output)

デフォルトの JSONL 出力では、選択されたターンごとに1つの JSON オブジェクトが出力されます：

```json
{"session":"session-id","project_dir":"-Users-example-project","cwd":"/Users/example/project","turn_index":1,"timestamp_local":"2026-09-17T11:04:07+09:00","human":"please fix this","prev_assistant":"What should I change?","prev_tools":[],"already_captured":false}
```

`--format md` では、セッションごとにグループ化された Markdown 形式で出力されます。`prev_assistant` は Unicode 文字数で適宜切り詰められます。Bash 呼び出しの場合、`prev_tools` には説明文（あれば）またはコマンドラインの先頭が含まれます（最大60文字）。`already_captured` は、次の人間の発言までに `/memory/` パスまたは末尾が `CLAUDE.md` や `AGENTS.md` のファイルを対象とした `Write` / `Edit` が行われていた場合に `true` になります。

## 人間の発言判定と定型文除外 (Boilerplate)

本当に人間が打ち込んだ発言であることの判定軸には `origin.kind == "human"` を使用しています。これにより、挿入されたシステムリマインダー、添付ファイルイベント、ツールの実行結果、サブエージェントへのプロンプトなどを壊れやすいテキスト解析なしに確実に除外します。

一部のランチャーは端末経由で定型プロンプトを流し込むため、ログ上は手入力と同じ `origin.kind == "human"` が付与されることがあります。そこで `tutor` は第2パスとして、3つ以上の異なるセッションで完全一致して現れた文字列を自動化による定型文（ボイラープレート）とみなして除外します。この閾値は設定可能で、除外された文字列と出現セッション数は stderr に報告されます。
