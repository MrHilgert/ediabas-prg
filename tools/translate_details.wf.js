export const meta = {
  name: 'translate-details',
  description: 'Translate BMW ECU detail/parameter labels DE→RU and DE→EN (Sonnet), chunked',
  phases: [{ title: 'Translate details', detail: 'per-chunk RU+EN agents, write TSV' }],
}

// args: { count: <total chunks>, limit?: <max chunks to process> }
// The runtime may hand `args` in as a JSON string — parse defensively.
const A = typeof args === 'string' ? JSON.parse(args) : (args || {})
const count = A.count ? A.count : 0
const limit = A.limit ? Math.min(A.limit, count) : count
const prefix = A.prefix ? A.prefix : 'det' // chunk-file prefix: det_/ch_/flt_

const idx = (i) => String(i).padStart(4, '0')

const RULES = (langName, extra) => `You are a professional automotive-diagnostics translator. Translate BMW ECU (EDIABAS/INPA) diagnostic strings from GERMAN to ${langName}. These are measurement/parameter labels and fault-code names/descriptions shown in a diagnostic tool's UI.

STRICT RULES:
- Translate ONLY German words into concise, standard ${langName} automotive-diagnostic terminology. Terse, unambiguous, technically correct. Shown in software — a wrong/vague translation misleads the user, so preserve exact meaning.
- Do NOT invent, add, explain or expand anything. No commentary, no glosses of your own.
- Copy VERBATIM (unchanged, in place): DTC codes (P0012, U11A5, 0x33FC, 4220, 49A3, hex), numbers, units (°C, V, 1/min, %), identifier tokens (DFC_*, DFES_DTCM..., INDEX_102_INJ, SLEEP_INDIKATION), CAN message names/IDs in parentheses (A_TEMP_RELATIVZEIT, 0x310), and established abbreviations (CAN, MOST, LIN, SCR, NOx, DAB, FM, AM, HID, ASIC, CPU, HW). Expand German abbreviations only when standard (SG → control unit / блок управления).
- ${extra}
- Keep original punctuation, separators (":", "-", "/", "-->", ";") and structure. "ein"/"aus" here mean on/off, not one/off.`

const OUTFMT = `OUTPUT: use the Write tool to write the output file. Each output line MUST be: <ORIGINAL SOURCE><TAB><TRANSLATION>. Column 1 is the EXACT source line copied verbatim; column 2 the translation; separated by ONE tab. Exactly one output line per input line, SAME order, SAME count. No numbering, headers, blank lines or code fences.`

const ruExtra = `If a line is ALREADY English, translate its meaning into Russian too (codes stay verbatim).`
const enExtra = `If a line is ALREADY English, keep it as-is (fix only obvious German fragments).`

function task(i, lang) {
  const inPath = `strings/chunks/${prefix}_${idx(i)}.txt`
  const outPath = `strings/loc/${lang}/${prefix}_${idx(i)}.tsv`
  const langName = lang === 'ru' ? 'RUSSIAN' : 'ENGLISH'
  const extra = lang === 'ru' ? ruExtra : enExtra
  const prompt = `${RULES(langName, extra)}

${OUTFMT}

STEPS:
1. Read the input file with the Read tool: ${inPath}
2. Translate every non-empty line per the rules.
3. Write the result with the Write tool to: ${outPath}
4. Reply with ONLY: OK <number-of-lines-written>

Do not print the translations in your reply — they go into the file.`
  return () =>
    agent(prompt, {
      label: `${lang}:det_${idx(i)}`,
      phase: 'Translate details',
      model: 'sonnet',
      agentType: 'general-purpose',
    }).then((r) => ({ i, lang, r }))
}

const tasks = []
for (let i = 0; i < limit; i++) {
  tasks.push(task(i, 'ru'))
  tasks.push(task(i, 'en'))
}

log(`translating ${limit}/${count} detail chunks × RU+EN = ${tasks.length} agents (Sonnet)`)
const results = (await parallel(tasks)).filter(Boolean)
const ok = results.filter((x) => x.r && /OK\s+\d+/i.test(x.r)).length
log(`done: ${ok}/${tasks.length} agents reported OK`)
return { requested: tasks.length, ok }
