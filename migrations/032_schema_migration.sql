-- 適用済みマイグレーションの記録。
--
-- なぜ要るか: 既存 DB は作り直さないと新しいマイグレーションに追いつかない。
-- 実際に `031_job_cancel.sql` がどの検証 DB にも当たっておらず、worker が
-- 毎 tick `column "cancel_requested_at" does not exist` で落ちた（2026-11-18）。
-- 以降は「当てたファイル名」を残し、`scripts/migrate-catchup.sh` が差分だけ当てる。
CREATE TABLE IF NOT EXISTS core.schema_migration (
    filename   text PRIMARY KEY,
    applied_at timestamptz NOT NULL DEFAULT now()
);

-- この表より前の履歴は個別には分からない。**032 に到達した時点で 001〜031 は
-- 流れている**（新規 DB はファイル順に流れるため）が、既存 DB では「人手で確認した」
-- という意味しか持たない。そのため `000_baseline` は 031 までを覆う印として置き、
-- 032 以降は 1 ファイル 1 行で記録する。
INSERT INTO core.schema_migration (filename)
VALUES ('000_baseline')
ON CONFLICT (filename) DO NOTHING;
