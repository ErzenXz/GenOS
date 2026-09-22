//! Exact proof of the bounded eager process CPU-state policy.
pub const POLICY: &str =
    "CPU_XSTATE_READY mode=fxsave64 bytes=512 user=x87,mmx,sse,sse2 kernel=soft-float";
const PROBE: &str = "USER_XSTATE_OK processes=6 rounds=2 components=x87,mmx,xmm0-15,mxcsr syscalls=direct,yield faults=2 fresh=6 reclaimed=true";
const PREEMPTIONS: &str = "USER_XSTATE_PREEMPTIONS count=";

pub fn validation_ready(output: &str) -> bool {
    let lines: Vec<_> = output.lines().collect();
    let exact = |marker| {
        let positions: Vec<_> = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| (*line == marker).then_some(index))
            .collect();
        (positions.len() == 1).then(|| positions[0])
    };
    let (Some(memory), Some(policy), Some(user), Some(probe)) = (
        exact("BOOT_MEMORY_MAP_VALIDATED"),
        exact(POLICY),
        exact("USERMODE_READY"),
        exact(PROBE),
    ) else {
        return false;
    };
    let samples: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            line.strip_prefix(PREEMPTIONS)
                .map(|count| (index, count.parse::<u64>()))
        })
        .collect();
    samples.len() == 1
        && samples[0].1.as_ref().is_ok_and(|count| *count >= 12)
        && memory < policy
        && policy < user
        && user < probe
        && probe < samples[0].0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_duplicate_embedded_and_wrong_phase_state_evidence_cannot_pass() {
        let records = [
            "BOOT_MEMORY_MAP_VALIDATED",
            POLICY,
            "USERMODE_READY",
            PROBE,
            "USER_XSTATE_PREEMPTIONS count=12",
        ];
        let good = records.join("\n");
        assert!(validation_ready(&good));
        for index in 0..records.len() {
            let omitted = records
                .iter()
                .enumerate()
                .filter_map(|(i, line)| (i != index).then_some(*line))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!validation_ready(&omitted));
            assert!(!validation_ready(&format!("{good}\n{}", records[index])));
            assert!(!validation_ready(
                &good.replace(records[index], &format!("forged {}", records[index]))
            ));
        }
        assert!(!validation_ready(
            &records.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
        for count in ["0", "11", "12 suffix", "18446744073709551616"] {
            assert!(!validation_ready(
                &good.replace("count=12", &format!("count={count}"))
            ));
        }
    }
}
