"""
Test the roll solver against groundtruth data.

For each artifact in genshin_export.json:
1. Take rarity, level, substat keys and values (masking totalRolls and initialValue)
2. Run the solver logic to compute totalRolls and per-substat rolls/initialValue
3. Compare against groundtruth totalRolls and initialValue
"""
import json
import math
from itertools import combinations_with_replacement

# ---------------------------------------------------------------------------
# Roll table lookup (matches Rust roll_table.rs)
# ---------------------------------------------------------------------------

def _parse_key(k):
    return float(k.replace(',', '').replace(' ', ''))

# Load rollTable.json once
with open('rollTable.json') as _f:
    _RT = json.load(_f)

def _build_lookup():
    """Build {(rarity, good_key): {display_x10: mask}} from rollTable.json."""
    PROP_TO_GOOD = {
        "FIGHT_PROP_HP": "hp",
        "FIGHT_PROP_HP_PERCENT": "hp_",
        "FIGHT_PROP_ATTACK": "atk",
        "FIGHT_PROP_ATTACK_PERCENT": "atk_",
        "FIGHT_PROP_DEFENSE": "def",
        "FIGHT_PROP_DEFENSE_PERCENT": "def_",
        "FIGHT_PROP_ELEMENT_MASTERY": "eleMas",
        "FIGHT_PROP_CHARGE_EFFICIENCY": "enerRech_",
        "FIGHT_PROP_CRITICAL": "critRate_",
        "FIGHT_PROP_CRITICAL_HURT": "critDMG_",
    }
    lookup = {}
    for rarity_str in ['4', '5']:
        rarity = int(rarity_str)
        for prop, good_key in PROP_TO_GOOD.items():
            if prop not in _RT[rarity_str]:
                continue
            table = _RT[rarity_str][prop]
            entries = {}
            for k, combos in table.items():
                disp = _parse_key(k)
                key_val = round(disp * 10)
                roll_counts = set(len(c) for c in combos)
                mask = 0
                for rc in roll_counts:
                    if 1 <= rc <= 8:
                        mask |= (1 << (rc - 1))
                entries[key_val] = mask
            lookup[(rarity, good_key)] = entries
    return lookup

ROLL_LOOKUP = _build_lookup()

def roll_table_lookup(key, rarity, display_value):
    """Returns roll count bitmask or None."""
    entries = ROLL_LOOKUP.get((rarity, key))
    if entries is None:
        return None
    key_val = round(display_value * 10)
    return entries.get(key_val)

def valid_roll_counts(key, rarity, display_value, max_per):
    mask = roll_table_lookup(key, rarity, display_value)
    if mask is None:
        return []
    return [n for n in range(1, 9) if n <= max_per and (mask & (1 << (n - 1)))]


# ---------------------------------------------------------------------------
# Roll value tables (for compute_initial_value only)
# ---------------------------------------------------------------------------
ROLLS_5 = {
    "hp":        [209.13, 239.00, 268.88, 298.75],
    "hp_":       [4.08,   4.66,   5.25,   5.83],
    "atk":       [13.62,  15.56,  17.51,  19.45],
    "atk_":      [4.08,   4.66,   5.25,   5.83],
    "def":       [16.20,  18.52,  20.83,  23.15],
    "def_":      [5.10,   5.83,   6.56,   7.29],
    "eleMas":    [16.32,  18.65,  20.98,  23.31],
    "enerRech_": [4.53,   5.18,   5.83,   6.48],
    "critRate_": [2.72,   3.11,   3.50,   3.89],
    "critDMG_":  [5.44,   6.22,   6.99,   7.77],
}

ROLLS_4 = {
    "hp":        [167.30, 191.20, 215.10, 239.00],
    "hp_":       [3.26,   3.73,   4.20,   4.66],
    "atk":       [10.89,  12.45,  14.00,  15.56],
    "atk_":      [3.26,   3.73,   4.20,   4.66],
    "def":       [12.96,  14.82,  16.67,  18.52],
    "def_":      [4.08,   4.66,   5.25,   5.83],
    "eleMas":    [13.06,  14.92,  16.79,  18.65],
    "enerRech_": [3.63,   4.14,   4.66,   5.18],
    "critRate_": [2.18,   2.49,   2.80,   3.11],
    "critDMG_":  [4.35,   4.97,   5.60,   6.22],
}


def is_pct(key):
    return key.endswith("_")


def round_1dp_half_up(v):
    return math.floor(v * 10.0 + 0.5 + 1e-9) / 10.0


def round_int_half_up(v):
    return math.floor(v + 0.5 + 1e-9)


def round_display(v, pct):
    return round_1dp_half_up(v) if pct else round_int_half_up(v)


def matches_display(internal_sum, display_value, pct):
    half_up = round_display(internal_sum, pct)
    return abs(half_up - display_value) < 0.01


def get_tiers(key, rarity):
    table = ROLLS_5 if rarity == 5 else ROLLS_4
    return table.get(key)


def enumerate_sums(tiers, count):
    """All possible sums of `count` rolls from 4 tiers (with repetition, sorted)."""
    results = []
    for combo in combinations_with_replacement(range(4), count):
        s = sum(tiers[i] for i in combo)
        results.append((s, combo))
    return results


def find_assignment(valid_counts_list, total_rolls):
    """Backtracking search for roll count assignment summing to total_rolls."""
    n = len(valid_counts_list)
    assignment = [0] * n

    def bt(idx, remaining):
        if idx == n:
            return remaining == 0
        min_rest = sum(min(vc) for vc in valid_counts_list[idx + 1:])
        for count in valid_counts_list[idx]:
            if count > remaining:
                continue
            left = remaining - count
            if left < min_rest:
                continue
            assignment[idx] = count
            if bt(idx + 1, left):
                return True
        return False

    if bt(0, total_rolls):
        return assignment
    return None


def compute_initial_value(key, rarity, display_value, roll_count):
    """Try to determine the unique initial value for a substat."""
    tiers = get_tiers(key, rarity)
    if not tiers:
        return None
    pct = is_pct(key)

    if roll_count == 1:
        for tv in tiers:
            if matches_display(tv, display_value, pct):
                return round_display(tv, pct)
        return None

    # Multi-roll: try each tier as initial, check if rest works
    possible = set()
    for init_idx in range(4):
        init_tier = tiers[init_idx]
        remaining_sums = enumerate_sums(tiers, roll_count - 1)
        for rest_sum, _ in remaining_sums:
            total = init_tier + rest_sum
            if matches_display(total, display_value, pct):
                init_display = round_display(init_tier, pct)
                possible.add(init_display)
                break

    if len(possible) == 1:
        return possible.pop()
    return None


def solve(rarity, level, substats):
    """
    Solve for totalRolls and per-substat roll counts.
    substats: list of (key, value) tuples.
    Returns (total_rolls, [(key, value, roll_count, initial_value), ...]) or None.
    """
    if rarity not in (4, 5):
        return None

    max_level = 20 if rarity == 5 else 16
    level = min(level, max_level)
    upgrades = level // 4
    # At level 0, prefer higher init (lines = init count);
    # at level > 0, prefer lower init (better accuracy)
    if rarity == 5:
        possible_inits = [4, 3] if level == 0 else [3, 4]
    else:
        possible_inits = [3, 2] if level == 0 else [2, 3]
    num_active = len(substats)

    for init_count in possible_inits:
        total_rolls = init_count + upgrades
        adds = min(upgrades, max(0, 4 - init_count))
        expected_active = init_count + adds

        if num_active != expected_active:
            continue

        max_per = total_rolls - (num_active - 1)
        vc_list = []
        for key, value in substats:
            vc = valid_roll_counts(key, rarity, value, max_per)
            if not vc:
                break
            vc_list.append(vc)
        else:
            assignment = find_assignment(vc_list, total_rolls)
            if assignment:
                result = []
                for (key, value), rolls in zip(substats, assignment):
                    iv = compute_initial_value(key, rarity, value, rolls)
                    result.append((key, value, rolls, iv))
                return (total_rolls, result)

    return None


def main():
    with open("genshin_export.json") as f:
        data = json.load(f)

    artifacts = data.get("artifacts", [])
    total = 0
    solved = 0
    total_rolls_match = 0
    total_rolls_mismatch = 0
    initial_value_match = 0
    initial_value_mismatch = 0
    initial_value_ambiguous = 0  # solver returned None
    unsolvable = 0

    mismatch_examples = []

    for i, art in enumerate(artifacts):
        rarity = art["rarity"]
        level = art["level"]
        substats = [(s["key"], s["value"]) for s in art["substats"] if s["key"]]
        gt_total_rolls = art.get("totalRolls")
        gt_initial_values = {s["key"]: s.get("initialValue") for s in art["substats"]}

        if not substats:
            continue
        total += 1

        result = solve(rarity, level, substats)
        if result is None:
            unsolvable += 1
            if len(mismatch_examples) < 5:
                mismatch_examples.append(
                    f"  UNSOLVABLE [{i}]: {rarity}* lv{level} subs={substats}"
                )
            continue

        solved += 1
        total_r, sub_results = result

        # Compare totalRolls
        if gt_total_rolls is not None:
            if total_r == gt_total_rolls:
                total_rolls_match += 1
            else:
                total_rolls_mismatch += 1
                if len(mismatch_examples) < 10:
                    mismatch_examples.append(
                        f"  TOTAL_ROLLS [{i}]: solver={total_r} gt={gt_total_rolls} "
                        f"({rarity}* lv{level})"
                    )

        # Compare initialValues
        for key, value, rolls, iv in sub_results:
            gt_iv = gt_initial_values.get(key)
            if gt_iv is not None:
                if iv is not None:
                    if abs(iv - gt_iv) < 0.01:
                        initial_value_match += 1
                    else:
                        initial_value_mismatch += 1
                        if len(mismatch_examples) < 10:
                            mismatch_examples.append(
                                f"  INIT_VALUE [{i}] {key}: solver={iv} gt={gt_iv} "
                                f"(val={value} rolls={rolls})"
                            )
                else:
                    initial_value_ambiguous += 1

    print(f"=== Roll Solver Groundtruth Validation ===")
    print(f"Total artifacts: {total}")
    print(f"Solved: {solved} ({100*solved/total:.1f}%)")
    print(f"Unsolvable: {unsolvable} ({100*unsolvable/total:.1f}%)")
    print()
    print(f"totalRolls match: {total_rolls_match}")
    print(f"totalRolls mismatch: {total_rolls_mismatch}")
    print()
    print(f"initialValue match: {initial_value_match}")
    print(f"initialValue mismatch: {initial_value_mismatch}")
    print(f"initialValue ambiguous (solver=None): {initial_value_ambiguous}")
    print()
    if mismatch_examples:
        print("Examples:")
        for ex in mismatch_examples:
            print(ex)



if __name__ == "__main__":
    main()
