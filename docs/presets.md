# Presets

What SqueezeIt is allowed to do to each kind of asset, and why.

Every rule here came out of breaking a real clothing pack on a real AMD machine. The
short version: **vehicles and props can take everything, clothing can only be resized,
hair should mostly be left alone.**

## How a file gets classified

Two things are worked out separately.

**The family** is per file. In order of trust:

1. Shader hash. `ped_hair_cutout_alpha`, `ped_hair_spiked` and `ped_fur` mean hair.
   `vehicle_*` means vehicle. `ped_*` and `cloth_*` mean clothing.
2. A dictionary holding a `vehicle_generic*` texture means vehicle.
3. The filename. Ped drawables are named `<pedmodel>^<component>_<index>[_<variant>]`,
   so `mp_f_freemode_01^hair_006_u.ydd` is hair and `^uppr_012_r.ydd` is clothing.
   `hair` and `berd` are hair, everything else in the component list is clothing.
4. Failing that, a name that reads as hair anyway: a stem starting `hair_`, ending
   `_hair`, or containing `_hair_`. That catches the loose `.ytd` a pack ships beside a
   drawable, which carries no component at all.
5. Anything left over is generic.

Shaders beat filenames, because a renamed file is common and a wrong shader hash is not.
Hair is the one exception: a file called `^hair_006_u.ydd` is treated as hair even when it
declares a cloth shader, because that is what those drawables actually are. `.ytd` files
have no shader group at all, so the name is all we get.

**The role** is per texture, from its name inside the dictionary, with a pixel-content
fallback for unlabelled maps. Roles are diffuse, normal, specular, hair, livery, weapon
and `script_rt`.

A single texture that reads as hair inside an otherwise normal dictionary gets the hair
treatment on its own.

## The presets

`Auto` is the default and dispatches per file. The named presets force one row onto
everything, for folders you already know are one kind of asset.

|                  | Clothing        | Hair & alpha       | Vehicles & props |
| ---------------- | --------------- | ------------------ | ---------------- |
| Diffuse cap      | 1024            | 1024, floor 512    | your limit       |
| Normal/spec cap  | 512             | 1024               | 1024             |
| Generate mips    | no (opt-in)     | **never**          | yes for props, opt-in for vehicles |
| Trim the mip tail| yes             | **no**             | yes              |
| Format changes   | never BC1       | **locked**         | aggressive       |
| Overdrive        | forced off      | forced off         | allowed          |
| GPU              | allowed         | forced off         | allowed          |
| Encoder floor    | Normal          | Normal             | your choice      |

The format rules above are enforced inside containers (`.ytd`, `.ydd`, `.ydr`, `.yft`).
Loose `.png`, `.jpg`, `.tga` and `.bmp` files pick their format from the texture role
alone, so an opaque loose diffuse still lands on BC1 whatever the preset says. Loose hair
and loose liveries are the exception and always get BC7.

Caps are ceilings, not targets. The effective limit is always
`min(your Max Size, the preset cap)`, so lowering the limit still works everywhere and
raising it only lifts families that have no ceiling of their own.

There is no separate strict hair preset any more. Reading, checking and writing a folder
back with no size changes is `--preset hair --max-res keep`, which is a size limit rather
than a preset. Old `settings.yaml` files naming `HairStrict` still load and land on `Hair`.

`Custom`, shown in the terminal UI as **No rules**, turns the family rules off entirely and
runs on the raw switches. It is the only way to do something the presets consider unsafe,
and it is on you if it breaks. It is also the one preset that overrules Safety: with no
family rules left to relax, the switch has nothing to say.

## Always protected

Under every preset, including Custom:

- `script_rt*` textures are never resized and never compressed. They are surfaces the
  game draws into while it runs. A broken one is repaired rather than left alone, which
  is a separate thing from being optimized; see `--script-rt`.
- Liveries, `_sign_` textures and weapon skins are never downscaled unless you pass
  `--include-liveries`. They are read close-up and at high zoom.
- Anything named with `--exclude` is left byte for byte as it was.
- Normal maps never get downgraded to BC1.
- No texture is ever placed across a page boundary. See "How containers get laid out".
- A container is never written if its header would promise more bytes than the graphics
  segment actually holds. That check runs on every path.

## Why hair and cloth are locked down

Three failure modes, each reproduced on real packs:

- **Hair strands break up at mid distance** once mipmaps are generated. Mips alone are
  enough. No format change is needed.
- **Fabric colours drift** with generated mips on cloth, and again with Overdrive, which
  puts a half-size normal map against a full-size diffuse.
- **A pack stops loading** when generated mips and a format change land on the same
  clothing texture.

The cause was never proven at the driver level. What is known is that the failures
correlate with alpha, a format change, and a rebuilt mip chain all reaching one texture
at once. The policy avoids the whole combination rather than pretending to know which
part is at fault.

BC1 is banned on anything ped-related for a separate reason: its 5:6:5 endpoints and
1-bit alpha wreck skin tones, fabric gradients and cutout edges. That one is not a guess.

## The opt-ins

Two switches let an asset do something the rules would otherwise refuse. The UI only
shows each of them when the current preset actually reaches a family it applies to.

**Safety, set to relaxed** (`--relaxed`). Lets clothing and vehicle textures grow a mip
chain they did not ship with. It is one switch rather than two because both halves are
the same decision. Relaxed does not skip the per-texture conditions below, it only stops
the family rules refusing outright.

For clothing, a texture may grow a chain only when *all* of these hold:

- no alpha anywhere in the top mip
- the output format matches the source format
- it shipped with no chain at all (`levels <= 1`)
- it is square, a power of two, and at least 128 wide
- it is in a `.ytd`

Every one of those is a factor that co-occurred with the failures. Requiring all of them
to be absent is the conservative reading. Anything that fails a condition falls back to
trimming the tail.

The `.ytd` condition is a policy decision, not a limitation: drawables are laid out
again and could grow a chain, but clothing lives mostly in drawables and clothing is
exactly what generated mips ruin. `mip_exception_applies` takes the container kind for
that reason, and a test pins it.

Vehicles carry no extra conditions; the family rule alone gates them, and a test pins
the behaviour.

**Shrink detail maps** (`--overdrive`). Halves the bumpiness and shininess maps against
the colour map they pair with. Off for clothing and hair under every preset, because a
half-size normal map against a full-size diffuse is one of the things that drifted fabric
colours.

Clothing follows whichever backend you picked. Hair always runs on the processor: the
two block compressors disagree most around cutout alpha, and hair is nothing but cutout
alpha.

## Mip levels

```
full     = rage_mip_levels(w, h)          // ilog2(long side) - 1
physical = physical_mip_levels(w, h)      // ilog2(long side) + 1

Preserve     -> source levels, clamped to physical
TrimTail     -> source levels, clamped to full
GenerateFull -> full, dictionaries only; drawables clamp to source levels
```

RAGE never samples the 2x2 and 1x1 tail, which is why `full` stops one short. The count
is keyed off the **long** side, so a 512x32 texture keeps the chain its 512 axis needs.

`Preserve` clamps to `physical` rather than `full` so a preserved chain carried onto a
smaller output does not overrun, while an unresized hair texture keeps its levels exactly
and passes straight through the "already optimal" early-out.

## How containers get laid out

A segment of a RAGE resource is not a flat buffer. It is a run of pages whose sizes are
encoded in the header, and the engine allocates those pages separately. Two pages next to
each other in the file are not next to each other in memory, so a block that crosses a
page boundary reads into whatever else happens to be there. That loads fine when the
allocator hands out adjacent pages and crashes when it does not, which is why crashes of
this kind follow the machine rather than the file.

`PageLayout::pack` in `core/src/rsc7/pages.rs` places every block inside one page and
picks the page classes to fit. Nothing is written until the finished container has been
read back and checked, and a container that fails is left untouched with the reason
logged.

`.ytd` containers are always laid out again, so they genuinely shrink. Identical texture
payloads inside one dictionary are folded onto a single offset while we are in there.

`.ydd`, `.ydr` and `.yft` are laid out again too, but only when every byte of their
graphics segment is accounted for. Vertex and index data live in the system segment, so a
drawable's graphics segment is usually textures and padding and nothing else. Usually is
not enough to move data on, so every graphics pointer in the container has to land on a
texture already known. If one does not, the file falls back to having its textures
patched into the slots they already hold. That still shrinks the file on disk, because
the leftover space is zeroed and zeros deflate away, but it frees nothing in memory.

A rebuild is also refused when the new layout would reserve more bytes than the original
while holding no more data, which happens on containers whose original layout was already
tight.

## What the numbers mean

Two figures exist and they answer different questions. Only one of them is reported.

**Reserved** is what the page layout makes the engine allocate. This is the memory figure,
and it is the one the report and the summary print. A drawable patched in place reports the
same reserved bytes before and after, because that is the truth.

**Held** is the sum of the texture payloads. It is what changed about the pixels. It is not
reported anywhere: it exists so a rebuild can refuse itself when it would reserve more
while holding no more. If you want the per-texture view, the `tracing` output has it.

## Known gaps

- The report counts files, not families. It cannot yet tell you "412 clothing textures
  resized, 0 mips generated". The per-file `tracing` output has it if you need it.
- Alpha-test coverage preservation on downscale is a known technique for alpha-tested
  geometry and would directly counter hair strand thinning. Not implemented.
- The 512 hair floor is a chosen number, not a measured threshold.
- `berd` is treated as hair. If a pack puts opaque masks under `^berd`, they get optimized
  more carefully than they need to be. That costs savings, not safety.
- The packer is first-fit-decreasing with a shrink pass, not optimal. It leaves gaps, and
  the only thing holding those gaps down is that a rebuild refuses itself when it would
  reserve more than the original for no more data.

## The backup vault

`--backup` moves each original into `.squeezeit-backup/` under the folder you pointed at,
records it in `squeezeit-manifest.tsv`, and writes the optimized file in its place.
`--restore` replays that manifest backwards and deletes the vault.

Every stored file gets a `.sqzbak` suffix appended after its real extension. This is not
cosmetic. The vault frequently ends up inside a resource's `stream/` directory, FiveM
enumerates that directory recursively and claims files by extension, and a dot-prefixed
folder name does not exempt it. Without the suffix the engine sees two assets with the
same internal name and picks one arbitrarily, which looks exactly like the optimization
having done nothing.

`collect_targets` skips a directory named `.squeezeit-backup` outright, so a second run
never re-processes originals. That match is on the default name only. A vault you sent
elsewhere with `--backup-dir` is skipped because it is outside the tree being walked, so
do not point `--backup-dir` at a folder inside the pack you are optimizing.

Restore accepts both suffixed and un-suffixed vault entries, so vaults written by older
builds still roll back.
