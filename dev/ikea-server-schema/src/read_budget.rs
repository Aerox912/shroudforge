//! All source content reads, including hash and parser inputs, use this path.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::io::Read;

pub const SOURCE_READ_CAP: u64 = 369_262_592;
const CHUNK: usize = 65_536;
#[derive(Clone, Copy)]
pub(crate) enum Phase {
    BeforeHash,
    Parse,
    AfterHash,
}
impl Phase {
    fn index(self) -> usize {
        match self {
            Self::BeforeHash => 0,
            Self::Parse => 1,
            Self::AfterHash => 2,
        }
    }
}
#[derive(Default, Clone, Copy)]
struct Counter {
    requested: u64,
    returned: u64,
    calls: u64,
    complete: bool,
}
pub(crate) struct SourceReads {
    lengths: [u64; 3],
    limit: u64,
    requested: u64,
    returned: u64,
    calls: u64,
    terminal: bool,
    counters: [[Counter; 3]; 3],
}
impl SourceReads {
    pub(crate) fn new(lengths: [u64; 3]) -> Self {
        Self {
            lengths,
            limit: SOURCE_READ_CAP,
            requested: 0,
            returned: 0,
            calls: 0,
            terminal: false,
            counters: [[Counter::default(); 3]; 3],
        }
    }
    pub(crate) fn fail(&mut self) {
        self.terminal = true;
        crate::diagnostics::source_read_counts(self.requested, self.returned, self.calls, true);
    }
    fn admitted(&self, slot: usize, phase: Phase) -> Result<u64> {
        ensure!(
            slot < 3 && !(slot == 2 && phase.index() == 1),
            "unadmitted source phase"
        );
        Ok(self.lengths[slot])
    }
    fn debit(&mut self, slot: usize, phase: Phase, amount: usize) -> Result<()> {
        ensure!(!self.terminal, "source reads terminal");
        let result = (|| {
            let size = self.admitted(slot, phase)?;
            let c = &self.counters[slot][phase.index()];
            ensure!(
                !c.complete && amount > 0 && amount <= CHUNK,
                "source request state"
            );
            let end = c
                .requested
                .checked_add(amount as u64)
                .ok_or_else(|| anyhow::anyhow!("source count overflow"))?;
            let total = self
                .requested
                .checked_add(amount as u64)
                .ok_or_else(|| anyhow::anyhow!("source total overflow"))?;
            ensure!(
                end <= size && total <= self.limit,
                "source requested read cap"
            );
            Ok(())
        })();
        if result.is_err() {
            self.fail();
            return result;
        }
        // This precedes the ONE actual read, including short and error returns.
        let c = &mut self.counters[slot][phase.index()];
        c.requested += amount as u64;
        c.calls += 1;
        self.requested += amount as u64;
        self.calls += 1;
        crate::diagnostics::source_read_counts(
            self.requested,
            self.returned,
            self.calls,
            self.terminal,
        );
        Ok(())
    }
    pub(crate) fn transfer<R: Read>(
        &mut self,
        reader: &mut R,
        slot: usize,
        phase: Phase,
        mut consume: impl FnMut(&[u8]),
    ) -> Result<()> {
        ensure!(!self.terminal, "source reads terminal");
        let result = (|| {
            let length = self.admitted(slot, phase)?;
            let c = &self.counters[slot][phase.index()];
            ensure!(
                !c.complete && c.requested == 0,
                "source phase already attempted"
            );
            let mut remaining = length;
            let mut buffer = [0u8; CHUNK];
            while remaining > 0 {
                let amount = remaining.min(CHUNK as u64) as usize;
                self.debit(slot, phase, amount)?;
                match reader.read(&mut buffer[..amount]) {
                    Ok(n) => {
                        ensure!(n <= amount, "invalid source read result");
                        self.counters[slot][phase.index()].returned += n as u64;
                        self.returned += n as u64;
                        crate::diagnostics::source_read_counts(
                            self.requested,
                            self.returned,
                            self.calls,
                            self.terminal,
                        );
                        // EOF, short success and Interrupted are terminal; never retry.
                        ensure!(n == amount, "short source read");
                        consume(&buffer[..n]);
                        remaining -= n as u64;
                    }
                    Err(_) => anyhow::bail!("source read error"),
                }
            }
            self.counters[slot][phase.index()].complete = true;
            Ok(())
        })();
        if result.is_err() {
            self.fail();
        }
        crate::diagnostics::source_read_counts(
            self.requested,
            self.returned,
            self.calls,
            self.terminal,
        );
        result
    }
    pub(crate) fn complete(&mut self) -> Result<Value> {
        ensure!(!self.terminal, "source reads terminal");
        for slot in 0..3 {
            for phase in [Phase::BeforeHash, Phase::Parse, Phase::AfterHash] {
                if slot == 2 && phase.index() == 1 {
                    continue;
                }
                let c = self.counters[slot][phase.index()];
                ensure!(
                    c.complete && c.requested == self.lengths[slot] && c.returned == c.requested,
                    "source phase incomplete"
                );
            }
        }
        Ok(self.observation())
    }
    pub(crate) fn observation(&self) -> Value {
        let names = [
            "enshrouded_server.exe",
            "enshrouded_server.kfc",
            "enshrouded_server.kfc_resources",
        ];
        let phases = ["before_hash", "parse_input", "after_hash"];
        let mut rows = Vec::new();
        for slot in 0..3 {
            for (p, phase) in phases.iter().enumerate() {
                let c = self.counters[slot][p];
                rows.push(json!({"file":names[slot],"phase":phase,"admitted":!(slot==2&&p==1),
                "requestedBytes":c.requested,"returnedBytes":c.returned,"readCalls":c.calls,"complete":c.complete}));
            }
        }
        json!({"maximumRequestedBytes":self.limit,"requestedBytes":self.requested,"returnedBytes":self.returned,
            "readCalls":self.calls,"terminal":self.terminal,"requestsDebitedBeforeIO":true,
            "shortOrErrorRead":"terminal_no_retry","EOFProbe":false,"returnedBytesMeaning":"bytes returned to application; error internal transfer UNKNOWN", "phases":rows})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    #[derive(Default)]
    struct Spy {
        requests: Vec<usize>,
        returned: u64,
        short: bool,
        error: bool,
    }
    impl Read for Spy {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.requests.push(out.len());
            if self.error {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "NO_RAW_ERROR_SENTINEL",
                ));
            }
            let n = if self.short { out.len() - 1 } else { out.len() };
            out[..n].fill(0x5a);
            self.returned += n as u64;
            Ok(n)
        }
    }
    #[test]
    fn read_a1_partial_last_chunk_and_exact_multiple_stop_without_eof() {
        for length in [0, 1, 65_536, 65_543] {
            let mut ledger = SourceReads::new([length, 0, 0]);
            let mut spy = Spy::default();
            let mut seen = 0;
            ledger
                .transfer(&mut spy, 0, Phase::BeforeHash, |b| seen += b.len() as u64)
                .unwrap();
            let expected = match length {
                0 => vec![],
                1 => vec![1],
                65_536 => vec![65_536],
                _ => vec![65_536, 7],
            };
            assert_eq!(spy.requests, expected);
            assert_eq!(seen, length);
            assert_eq!(ledger.requested, length);
            assert_eq!(ledger.returned, length);
            assert!(
                ledger
                    .transfer(&mut spy, 0, Phase::BeforeHash, |_| {})
                    .is_err()
            );
            assert_eq!(spy.requests, expected); // No repeated phase and no zero-length/EOF read.
        }
    }
    #[test]
    fn read_a1_short_and_error_are_debited_then_terminal_across_files_and_phases() {
        for error in [false, true] {
            let mut ledger = SourceReads::new([100, 100, 100]);
            let mut spy = Spy {
                short: !error,
                error,
                ..Spy::default()
            };
            assert!(
                ledger
                    .transfer(&mut spy, 0, Phase::BeforeHash, |_| panic!(
                        "partial payload consumed"
                    ))
                    .is_err()
            );
            assert_eq!(ledger.requested, 100);
            assert_eq!(ledger.returned, if error { 0 } else { 99 });
            assert_eq!(ledger.calls, 1);
            assert!(ledger.terminal);
            let before = spy.requests.clone();
            assert!(
                ledger
                    .transfer(&mut spy, 0, Phase::BeforeHash, |_| {})
                    .is_err()
            );
            assert!(ledger.transfer(&mut spy, 1, Phase::Parse, |_| {}).is_err());
            assert_eq!(spy.requests, before);
            assert!(ledger.complete().is_err());
            assert!(
                !ledger
                    .observation()
                    .to_string()
                    .contains("NO_RAW_ERROR_SENTINEL")
            );
        }
    }
    #[test]
    fn read_a1_aggregate_refusal_precedes_io_and_includes_parse_requests() {
        let mut ledger = SourceReads::new([7, 5, 3]);
        ledger.limit = 20;
        let mut spy = Spy::default();
        ledger
            .transfer(&mut spy, 0, Phase::BeforeHash, |_| {})
            .unwrap();
        ledger
            .transfer(&mut spy, 1, Phase::BeforeHash, |_| {})
            .unwrap();
        ledger
            .transfer(&mut spy, 2, Phase::BeforeHash, |_| {})
            .unwrap();
        // 15 already requested; parse7 cannot be admitted even though each file fits.
        assert!(ledger.transfer(&mut spy, 0, Phase::Parse, |_| {}).is_err());
        assert_eq!(spy.requests, vec![7, 5, 3]);
        assert_eq!(ledger.requested, 15);
        assert!(ledger.terminal);
        assert_eq!(ledger.calls, 3);
        assert!(
            ledger
                .transfer(&mut spy, 1, Phase::AfterHash, |_| {})
                .is_err()
        );
        assert_eq!(spy.requests.len(), 3);
    }
    #[test]
    fn read_a1_full_three_file_two_hash_parse_schedule_exact_cap() {
        let lengths = crate::acquire::PINS.map(|pin| pin.1);
        let mut ledger = SourceReads::new(lengths);
        let mut spy = Spy::default();
        for slot in 0..3 {
            ledger
                .transfer(&mut spy, slot, Phase::BeforeHash, |_| {})
                .unwrap();
        }
        for slot in 0..2 {
            ledger
                .transfer(&mut spy, slot, Phase::Parse, |_| {})
                .unwrap();
        }
        for slot in 0..3 {
            ledger
                .transfer(&mut spy, slot, Phase::AfterHash, |_| {})
                .unwrap();
        }
        let report = ledger.complete().unwrap();
        assert_eq!(report["requestedBytes"], SOURCE_READ_CAP);
        assert_eq!(report["returnedBytes"], SOURCE_READ_CAP);
        assert_eq!(spy.requests.iter().sum::<usize>() as u64, SOURCE_READ_CAP);
        assert!(spy.requests.iter().all(|&n| n > 0 && n <= CHUNK));
        assert_eq!(ledger.calls, 5640);
        assert!(ledger.transfer(&mut spy, 2, Phase::Parse, |_| {}).is_err());
        assert_eq!(ledger.requested, SOURCE_READ_CAP);
    }
}
