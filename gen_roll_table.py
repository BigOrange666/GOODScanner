"""Generate Rust lookup table from rollTable.json"""
import json

with open('rollTable.json') as f:
    rt = json.load(f)

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

def parse_key(k):
    return float(k.replace(',', '').replace(' ', ''))

# For the solver, we need: given (rarity, stat_key, display_value) -> set of valid roll counts
# Store as sorted arrays of (display_value_x10: i32, roll_count_mask: u8)
# where roll_count_mask has bit (n-1) set if n rolls can produce that value.

STAT_ORDER = ["atk", "atk_", "critDMG_", "critRate_", "def", "def_", "eleMas", "enerRech_", "hp", "hp_"]

lines = []

for rarity_str in ['4', '5']:
    rarity = int(rarity_str)
    for good_key in STAT_ORDER:
        # Find the prop key
        prop = [p for p, g in PROP_TO_GOOD.items() if g == good_key][0]
        if prop not in rt[rarity_str]:
            continue
        table = rt[rarity_str][prop]
        entries = []
        for k, combos in table.items():
            disp = parse_key(k)
            roll_counts = set(len(c) for c in combos)
            key = round(disp * 10)
            mask = 0
            for rc in roll_counts:
                if 1 <= rc <= 8:
                    mask |= (1 << (rc - 1))
            entries.append((key, mask))
        entries.sort()

        suffix = good_key.rstrip('_').upper()
        if good_key.endswith('_'):
            suffix += "_PCT"
        var_name = f"RT_{rarity}_{suffix}"
        lines.append(f"const {var_name}: &[(i32, u8)] = &[")
        for key, mask in entries:
            lines.append(f"    ({key}, 0b{mask:08b}),")
        lines.append("];")

lines.append("")
# Generate the dispatch function
lines.append("fn roll_table(key: &str, rarity: i32) -> Option<&'static [(i32, u8)]> {")
lines.append("    match (rarity, key) {")
for rarity_str in ['4', '5']:
    rarity = int(rarity_str)
    for good_key in STAT_ORDER:
        suffix = good_key.rstrip('_').upper()
        if good_key.endswith('_'):
            suffix += "_PCT"
        var_name = f"RT_{rarity}_{suffix}"
        lines.append(f'        ({rarity}, "{good_key}") => Some({var_name}),')
lines.append("        _ => None,")
lines.append("    }")
lines.append("}")

print('\n'.join(lines))
