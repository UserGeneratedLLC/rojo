---
name: Compact JSON5 arrays
overview: Arrays with <12 non-compound elements render on one line. Recursive -- inner arrays compact independently.
todos:
  - id: modify-array-arm
    content: "Modify the Array arm of write_to() in src/json.rs: inline if <12 elements and no element is Array/Object"
    status: completed
  - id: snapshot-tests
    content: Run cargo test, then cargo insta review to update affected snapshots
    status: completed
isProject: false
---

# Compact JSON5 Array Formatting

## Rule

An array renders inline if:

1. Fewer than 12 direct elements
2. No direct element is an `Array` or `Object`

Format: `[ elem, elem, elem ]` (spaces inside brackets). Recursive -- each nested array independently decides.

## Change

One file: [src/json.rs](src/json.rs), the Array arm of `Json5Value::write_to()` (lines 154-169). ~10 lines added.

---

## Expected Output Examples

### Color3 (3 floats)

Before:

```json5
Color3: [
  0.41568630933761597,
  0,
  1
]
```

After:

```json5
Color3: [ 0.41568630933761597, 0, 1 ]
```

### NumberRange (2 floats)

Before:

```json5
Lifetime: {
  NumberRange: [
    0.25,
    0.6000000238418579
  ]
}
```

After:

```json5
Lifetime: {
  NumberRange: [ 0.25, 0.6000000238418579 ]
}
```

### Tags (string array)

Before:

```json5
Tags: [
  "SoundGroupEffects"
]
```

After:

```json5
Tags: [ "SoundGroupEffects" ]
```

Multiple tags:

```json5
Tags: [ "_VFX", "Collectible", "PowerUp" ]
```

### CFrame (object with position + orientation)

Before:

```json5
CFrame: {
  position: [
    -0.09659165,
    1.517803,
    -0.83597946
  ],
  orientation: [
    [
      0.10630996,
      -0.0024746126,
      -0.9943304
    ],
    [
      0.021991052,
      0.999758,
      -0.0021534874
    ],
    [
      0.9941102,
      -0.021623673,
      0.1063388
    ]
  ]
}
```

After:

```json5
CFrame: {
  position: [ -0.09659165, 1.517803, -0.83597946 ],
  orientation: [
    [ 0.10630996, -0.0024746126, -0.9943304 ],
    [ 0.021991052, 0.999758, -0.0021534874 ],
    [ 0.9941102, -0.021623673, 0.1063388 ]
  ]
}
```

- `position` (3 numbers) -> inline
- `orientation` (3 arrays) -> multi-line because elements are arrays
- Each inner orientation row (3 numbers) -> inline

### UDim2 (2 arrays of 2 numbers)

Before:

```json5
Size: [
  [
    1,
    0
  ],
  [
    0,
    100
  ]
]
```

After:

```json5
Size: [
  [ 1, 0 ],
  [ 0, 100 ]
]
```

Outer has array elements -> multi-line. Inner arrays (2 numbers each) -> inline.

### Rect (2 Vector2s)

Before:

```json5
ImageRectOffset: [
  [
    100,
    100
  ],
  [
    200,
    200
  ]
]
```

After:

```json5
ImageRectOffset: [
  [ 100, 100 ],
  [ 200, 200 ]
]
```

### ColorSequence keypoints (array of objects -- NO CHANGE)

Before and after (objects always expand):

```json5
Color: {
  ColorSequence: {
    keypoints: [
      {
        color: [ 0.364705890417099, 0.11372549086809158, 1 ],
        time: 0
      },
      {
        color: [ 0.364705890417099, 0.11372549086809158, 1 ],
        time: 1
      }
    ]
  }
}
```

- `keypoints` array has objects -> stays multi-line
- `color` arrays inside each keypoint (3 numbers) -> inline

### NumberSequence with 20 keypoints (array of objects -- NO CHANGE)

Stays fully expanded. Both because elements are objects AND count >= 12.

### Full file: Texture.model.json5

Before:

```json5
{
  className: "Texture",
  properties: {
    Color3: [
      0.41568630933761597,
      0,
      1
    ],
    Face: "Top",
    StudsPerTileU: 200,
    StudsPerTileV: 200,
    TextureContent: "rbxassetid://10025668350",
    Transparency: 0.6000000238418579
  }
}
```

After:

```json5
{
  className: "Texture",
  properties: {
    Color3: [ 0.41568630933761597, 0, 1 ],
    Face: "Top",
    StudsPerTileU: 200,
    StudsPerTileV: 200,
    TextureContent: "rbxassetid://10025668350",
    Transparency: 0.6000000238418579
  }
}
```

### Full file: Water.model.json5

Before:

```json5
{
  attributes: {
    Rojo_Ref_SoundGroup: "ServerStorage/__server_muted.model.json5"
  },
  className: "Sound",
  properties: {
    AudioContent: "rbxassetid://9120549564",
    Looped: true,
    Playing: true,
    SourceAssetId: 9120549564,
    Tags: [
      "SoundGroupEffects"
    ],
    Volume: 3
  }
}
```

After:

```json5
{
  attributes: {
    Rojo_Ref_SoundGroup: "ServerStorage/__server_muted.model.json5"
  },
  className: "Sound",
  properties: {
    AudioContent: "rbxassetid://9120549564",
    Looped: true,
    Playing: true,
    SourceAssetId: 9120549564,
    Tags: [ "SoundGroupEffects" ],
    Volume: 3
  }
}
```

### Edge: empty array (NO CHANGE)

```json5
Tags: []
```

### Edge: 12+ primitive elements (stays multi-line)

```json5
SomeList: [
  1,
  2,
  3,
  4,
  5,
  6,
  7,
  8,
  9,
  10,
  11,
  12
]
```

