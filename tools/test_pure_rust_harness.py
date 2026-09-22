import tempfile
from pathlib import Path
import unittest

from test_pure_rust import MODEL_PROOF, validate_model_log, validate_test_log


class PureRustHarnessTests(unittest.TestCase):
    def test_miri_requires_one_successful_nonempty_complete_scope(self):
        good = "test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 171 filtered out; finished in 7.18s\n"
        validate_test_log(good, 14)
        for bad in ["", good + good, "forged " + good, good.replace("14 passed", "0 passed"),
                    good.replace("0 failed", "1 failed"), good.replace("0 ignored", "1 ignored"),
                    good + "error: Undefined Behavior\n", good + "error: unsupported operation\n"]:
            with self.subTest(log=bad), self.assertRaises(ValueError):
                validate_test_log(bad, 14)

    def test_model_requires_every_exhaustive_depth_and_exact_final_counts(self):
        records = [f"OWNERSHIP_MODEL_DEPTH_OK depth={d} sequences={12 ** d}" for d in range(1, 7)]
        records.append(MODEL_PROOF + "746")
        good = "\n".join(records) + "\n"
        validate_model_log(good)
        for record in records:
            for bad in [good.replace(record + "\n", ""), good + record + "\n",
                        good.replace(record, "forged " + record)]:
                with self.subTest(log=bad), self.assertRaises(ValueError):
                    validate_model_log(bad)
        for bad in [good.replace("sequences=3257436", "sequences=3257435"),
                    good.replace("merged_states=0", "merged_states=1"),
                    good + "OWNERSHIP_MODEL_FAILED\n", "\n".join(reversed(records))]:
            with self.subTest(log=bad), self.assertRaises(ValueError):
                validate_model_log(bad)


if __name__ == "__main__":
    unittest.main()
