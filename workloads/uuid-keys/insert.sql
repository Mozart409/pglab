-- One row per transaction; the key comes from the column default, so the same
-- script drives every key type.
insert into items (payload) values (md5(random()::text));
