-- A stake move is one event: keep where it came from, and drop the parts the chain also reports.
ALTER TABLE events ADD COLUMN from_hotkey text, ADD COLUMN from_netuid int;

-- Existing moves: the source is the StakeRemoved reported just before them.
UPDATE events m SET from_hotkey = r.hotkey, from_netuid = r.netuid
FROM events r
WHERE m.kind = 'stake_moved' AND m.from_hotkey IS NULL
  AND r.block = m.block AND r.address = m.address AND r.kind = 'stake_removed' AND r.idx = m.idx - 2;

-- ...and the StakeRemoved + StakeAdded parts of each move.
DELETE FROM events p USING events m
WHERE m.kind = 'stake_moved' AND p.block = m.block AND p.address = m.address
  AND ((p.kind = 'stake_removed' AND p.idx = m.idx - 2 AND p.hotkey = m.from_hotkey)
    OR (p.kind = 'stake_added' AND p.idx = m.idx - 1 AND p.hotkey = m.hotkey));

-- The money leg of a stake / unstake (a transfer to / from the stake pool account) is not a payment.
DELETE FROM events t USING events s
WHERE t.block = s.block AND t.address = s.address AND t.amount = s.amount
  AND ((t.kind = 'transfer_out' AND s.kind = 'stake_added') OR (t.kind = 'transfer_in' AND s.kind = 'stake_removed'));
