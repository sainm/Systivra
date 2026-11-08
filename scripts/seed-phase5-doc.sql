\set ON_ERROR_STOP on

INSERT INTO corpus.document
  (id, project_id, doc_type, title, source_path, current_revision_id)
VALUES
  ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
   '22222222-2222-4222-8222-222222222222',
   'DETAILED_DESIGN',
   'UserService 呼び出し仕様',
   'phase5/user-service.md',
   NULL)
ON CONFLICT (id) DO NOTHING;

INSERT INTO corpus.document_revision
  (id, document_id, source_revision, content_hash, object_key,
   file_size, mime_type, status, source_encoding, encoding_confidence, acquired_at)
VALUES
  ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
   'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
   'phase5-r1',
   '\x10',
   'raw/phase5/user-service.md',
   256,
   'text/markdown',
   'CURRENT',
   'UTF-8',
   1.0,
   now())
ON CONFLICT (id) DO NOTHING;

UPDATE corpus.document
   SET current_revision_id = 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'
 WHERE id = 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa';

INSERT INTO corpus.chunk
  (id, project_id, document_id, revision_id, chunk_type, ordinal,
   text, tokenized, token_count, char_count, content_hash, locator,
   node_path, metadata, is_current)
VALUES
  ('cccccccc-cccc-4ccc-8ccc-cccccccccccc',
   '22222222-2222-4222-8222-222222222222',
   'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
   'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
   'TEXT',
   1,
   'UserService.save は UserEditAction.update から呼び出され、住所を AddressService.validate で確認する。',
   'UserService save UserEditAction update AddressService validate',
   8,
   69,
   '\x20',
   '{"kind":"text","section":"UserService 呼び出し仕様","paragraph":1}'::jsonb,
   ARRAY['DOCUMENT','SECTION:1'],
   '{"phase":"phase5-eval"}'::jsonb,
   true)
ON CONFLICT (id, project_id) DO NOTHING;

INSERT INTO corpus.evidence
  (id, project_id, source_type, document_id, revision_id, chunk_id,
   locator, quote, quote_hash, resolution, confidence)
VALUES
  ('dddddddd-dddd-4ddd-8ddd-dddddddddddd',
   '22222222-2222-4222-8222-222222222222',
   'CHUNK_TEXT',
   'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
   'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
   'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
   '{"kind":"text","section":"UserService 呼び出し仕様","paragraph":1}'::jsonb,
   'UserService.save は UserEditAction.update から呼び出され、住所を AddressService.validate で確認する。',
   '\x30',
   'RESOLVED',
   1.0)
ON CONFLICT (id) DO NOTHING;
