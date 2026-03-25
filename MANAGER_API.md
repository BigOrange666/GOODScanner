# Artifact Manager HTTP API

Server: `http://127.0.0.1:{port}` (default 8765)

## Endpoints

### `GET /health`

```json
{"status":"ok","enabled":true}
```

`enabled: false` means the manager is paused — `/manage` will return 503.

### `POST /manage`

#### Request

```json
{
  "instructions": [
    {
      "id": "client-tracking-id",
      "target": {
        "setKey": "GladiatorsFinale",
        "slotKey": "flower",
        "rarity": 5,
        "level": 20,
        "mainStatKey": "hp",
        "substats": [
          {"key": "critRate_", "value": 3.9},
          {"key": "critDMG_", "value": 7.8}
        ]
      },
      "changes": {
        "lock": true,
        "location": "Furina"
      }
    }
  ]
}
```

#### Fields

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `id` | string | yes | Client-assigned ID, returned in results |
| `target.setKey` | string | yes | GOOD v3 PascalCase (e.g. `"GladiatorsFinale"`) |
| `target.slotKey` | string | yes | `flower` `plume` `sands` `goblet` `circlet` |
| `target.rarity` | int | yes | 1–5 |
| `target.level` | int | yes | 0–20 |
| `target.mainStatKey` | string | yes | GOOD v3 stat key (e.g. `"hp"`, `"atk_"`) |
| `target.substats` | array | yes | `[{key, value}]`, order-independent matching |
| `changes.lock` | bool? | no | Set lock state. Omit or `null` to skip. |
| `changes.location` | string? | no | GOOD character key to equip to. `""` = unequip. Omit or `null` to skip. |

At least one of `lock` or `location` must be present.

#### Response

```json
{
  "results": [
    {
      "id": "client-tracking-id",
      "status": "success",
      "detail": null
    }
  ],
  "summary": {
    "total": 1,
    "success": 1,
    "already_correct": 0,
    "not_found": 0,
    "errors": 0,
    "aborted": 0
  }
}
```

#### Status Values

| Status | Meaning |
|--------|---------|
| `success` | Applied |
| `already_correct` | Already in desired state |
| `not_found` | No matching artifact found |
| `invalid_input` | Bad data (empty keys, out-of-range values) |
| `ocr_error` | OCR identification failed |
| `ui_error` | Game UI interaction failed |
| `aborted` | User cancelled (right-click) |
| `skipped` | Skipped (earlier failure or abort) |

#### HTTP Errors

| Code | Meaning |
|------|---------|
| 400 | Malformed JSON |
| 503 | Manager paused |
| 404 | Unknown endpoint |

## Examples

Lock one artifact:

```json
{
  "instructions": [{
    "id": "1",
    "target": {
      "setKey": "EmblemOfSeveredFate",
      "slotKey": "sands",
      "rarity": 5,
      "level": 20,
      "mainStatKey": "enerRech_",
      "substats": [
        {"key": "critRate_", "value": 10.5},
        {"key": "critDMG_", "value": 19.4},
        {"key": "atk_", "value": 5.8},
        {"key": "hp", "value": 508}
      ]
    },
    "changes": {"lock": true}
  }]
}
```

Equip to character:

```json
{
  "instructions": [{
    "id": "2",
    "target": {
      "setKey": "GladiatorsFinale",
      "slotKey": "flower",
      "rarity": 5,
      "level": 20,
      "mainStatKey": "hp",
      "substats": []
    },
    "changes": {"location": "Furina"}
  }]
}
```

Unequip:

```json
{
  "instructions": [{
    "id": "3",
    "target": {
      "setKey": "GladiatorsFinale",
      "slotKey": "flower",
      "rarity": 5,
      "level": 20,
      "mainStatKey": "hp",
      "substats": []
    },
    "changes": {"location": ""}
  }]
}
```

Lock + equip in one instruction:

```json
{
  "instructions": [{
    "id": "4",
    "target": {
      "setKey": "NoblesseOblige",
      "slotKey": "goblet",
      "rarity": 5,
      "level": 20,
      "mainStatKey": "pyro_dmg_",
      "substats": [
        {"key": "critRate_", "value": 7.0},
        {"key": "critDMG_", "value": 14.0}
      ]
    },
    "changes": {"lock": true, "location": "HuTao"}
  }]
}
```

Batch (multiple instructions):

```json
{
  "instructions": [
    {"id": "a", "target": {...}, "changes": {"lock": true}},
    {"id": "b", "target": {...}, "changes": {"lock": false}},
    {"id": "c", "target": {...}, "changes": {"location": "Nahida"}}
  ]
}
```

All instructions execute sequentially. Invalid ones are filtered and reported individually; valid ones still run.
