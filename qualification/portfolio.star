def assert_pass(name, artifact):
    assert_eq(artifact["result"], "pass", name + " did not pass")

def assert_equivalent(results, compare):
    baseline = results[0]
    for candidate in results[1:]:
        for field in compare:
            assert_eq(candidate[field], baseline[field], "divergent field: " + field)

def assert_reduction(proofs):
    assert_eq(len(proofs), 3)
    for proof in proofs:
        assert_true(proof["final_reproducible"])
        assert_eq(len(proof["minimized"]["source"]), 1)
        assert_eq(len(proof["minimized"]["flags"]), 1)
        assert_eq(len(proof["minimized"]["inputs"]), 1)

assert_pass("RISC-V CPU", riscv_cpu)
assert_pass("Arm CPU", arm_cpu)
assert_pass("Xtensa CPU", xtensa_cpu)
assert_pass("three-family reduction", reduction)

assert_reduction(reduction["proofs"])

assert_equivalent(
    [riscv_cpu, arm_cpu, xtensa_cpu],
    ["result"],
)

True
