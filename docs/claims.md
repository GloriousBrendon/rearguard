# Claims register

Last checked: 1 October 2026. Version 1.

Every claim made about Rearguard, its sources or its results is listed here with how well it is supported. The rule: nothing goes into a pitch, README or announcement unless it is verified against a primary source or is our own measurement from real players. Results from simulation must always be labelled as simulated.

## Status labels

- **Verified (primary):** the vendor's or the authors' own publication was found. Read it in full before quoting details.
- **Verified (secondary):** reported by a news site or community post that cites the vendor. Trace it to the original before use.
- **Partly verified:** some of the claim is supported and some is not.
- **Corrected:** the claim in the concept record was wrong or was being misread, and the correction is given.
- **Own measurement (simulation / benchmark / human):** produced by this project. Simulation results are not real-world results.
- **Not yet measured:** a planned measurement has not run.
- **Unverified:** no source was checked.
- **Do not claim:** must not be said.

## Summary

53 claims. Unverified: 15; Verified (secondary): 10; Partly verified: 6; Verified (primary): 6; Own measurement (simulation): 4; Do not claim: 4; Corrected: 2; Not yet measured: 2; Own measurement: 2; Own measurement (benchmark): 1; Own measurement (human): 1.

## A. The problem Rearguard addresses

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| A1 | Kernel anti-cheat runs closed-source code with full access to the player's machine. | Unverified | General knowledge. Riot describes Vanguard as a kernel-level client, but the point was not checked against a primary source. | Cite a vendor description before using it. |
| A2 | On Linux anyone can compile their own kernel, so kernel anti-cheat cannot trust the kernel. | Unverified | A technical argument, not a sourced fact. | Present as an argument, or cite a vendor statement. |
| A3 | EAC and BattlEye can run under Proton when the developer opts in. | Verified (secondary) | Epic announced Linux, Wine and Proton support for Easy Anti-Cheat in September 2021, and BattlEye followed the next day with Proton support, opt-in for developers. Reported by PC Gamer, Phoronix, GamingOnLinux and It's FOSS. Proton 7.0 supports EAC when the game has enabled a Linux module (Linuxiac). | Re-check the current state (2021 sources). Read the Epic announcement itself. How many studios opted in has not been checked. |
| A4 | External cheats (DMA cards, capture-card aimbots) never touch the game PC, and cheats keep bypassing kernel anti-cheat. | Partly verified | FACEIT's own figures, as reported by Tech Times, say AI and DMA cheats made up 40% of its cheating bans in May 2026. Witschel and Wressnegger (2020) show an adaptive aimbot evading VAC, VACnet and Overwatch. No primary statistic on kernel anti-cheat bypasses was found. | Cite the two sources with attribution. Avoid the blanket statement. |
| A5 | In free-to-play games banned accounts are cheap to replace, so studios prefer prevention to punishment. | Unverified | An opinion from the design discussion. | Remove or soften before the pitch. |

## B. Prior art and related work

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| B1 | Activision's Ricochet ships decoy players (Hallucinations) that clone a real player's movement and are visible only to cheats, placed near flagged suspects. | Verified (primary) | Activision's Ricochet Season 04 update (callofduty.com, June 2023) describes Hallucinations as an active mitigation. Found through search; the full post was not read. | Read the full post before citing the details. |
| B2 | Invisibility Cloak (USENIX Security 2024) adds imperceptible perturbations to game visuals so AI aimbots misread them, tested in CrossFire and CS2. | Verified (primary) | USENIX abstract: the authors call it the first proactive defence against visual game cheating, and report that deploying it online in both games eliminated almost all aiming and shooting behaviour associated with aimbots. | Read the paper before citing results or limits. |
| B3 | AimTrap (arXiv, 2026) combines camouflage and honeypot textures against visual aimbots with low false positives and negligible runtime cost. | Partly verified | Preprint titled 'Shoot the Honey, Cloak the Player'. Its abstract reports 85.1% defence success for the camouflage mechanism and 96.9% for the honeypot mechanism. A summary says it matched a behavioural detector (XGuardian) with perfect recall and precision in a 40-match study; its trajectory-level false-positive rate was about 0.33%, with a much lower player-level figure computed from an aggregation rule rather than measured. | Preprint, not peer reviewed. Read in full. Quote the measured rate, not the computed one. |
| B4 | Adversarial patches as honeytokens against visual aimbots, with a Fortnite proof of concept (arXiv 2606.07650). | Unverified | Listed in the concept record. Not opened during this check. | Read before citing. |
| B5 | An adaptive aimbot that mimics user behaviour can evade state-of-the-art aimbot detectors (Witschel and Wressnegger, 2020). | Verified (primary) | Read in full. An aimbot that improved a player's hit rate by about 5% went unflagged over 60 matches on official CS:GO servers with VAC, VACnet and Overwatch active. Two players, one game, 2020. The paper also repeats Valve's 80 to 95% figure and calls it a detection rate (see B8). | Cite as a small study. Do not generalise to current systems. |
| B6 | Rust community thread proposing randomised recoil, with objections that noticeable randomness hurts legitimate players. | Unverified | Link in the concept record (rust.nolt.io/7818). Not opened. | Read before citing, or drop it. |
| B7 | Valve's VACnet analyses matches with deep learning and sends suspicious cases to Overwatch. | Verified (secondary) | Valve's John McDonald, GDC 2018, as quoted by PC Gamer via ResetEra and summarised by a Harvard D3 student report: about 150,000 matches scored a day. The GDC talk itself was not read. | Read the talk. What VACnet analyses was not verified, so the record's description of it as input-versus-screen correlation is unsupported. |
| B8 | VACnet 'detects 80 to 95% of cheats'. | Corrected | The 80 to 95% figure is the conviction rate of cases VACnet submitted to Overwatch, against 15 to 30% for human-submitted cases. That measures the precision of submitted cases, not the share of cheaters caught. Some sources, including the Witschel and Wressnegger paper, call it a detection rate. | Never quote it as a detection rate. |
| B9 | FACEIT Minerva is an example of server-side input-versus-screen correlation. | Corrected | Minerva launched in 2019 with Google and Jigsaw as a chat-toxicity AI. Version 0.1 handled chat only and was described as not yet trained to detect cheaters. A 2024 Arab News piece says it also provides anti-cheat, with no technical detail. | Remove Minerva as an example from the concept record. For FACEIT's input work, cite Human Input Detection (B13). |
| B10 | Valve's Trust Factor matches likely cheaters together and away from trusted players. | Verified (secondary) | Introduced in November 2017. Valve's blog, quoted by Destructoid, says it uses observed behaviours and Steam account attributes, including time played and how often a player is reported for cheating. A patent description reported by a secondary site says it matches likely cheaters together. Valve does not publish the factors. | Cite Valve's Counter-Strike blog post directly. |
| B11 | Valorant's Fog of War withholds enemy positions from the client until they are needed. | Verified (primary) | Riot's 'Demolishing Wallhacks with VALORANT's Fog of War' (riotgames.com) says the server withholds enemy positions until a client needs to display them, based on League's system. A Riot developer said a nearby sound can also trigger the location being sent (secondary report). | Cite the Riot post. Fits our culling task (2.5). |
| B12 | Open-source server-side culling already exists. | Verified (secondary) | The CS2FOW plugin for CS2 (GameRiv article) and the CornerCulling repository on GitHub, which describes ray-cast based occlusion culling. | Check their licences and designs before 2.5. This affects any novelty claim about culling. |
| B13 | FACEIT's Human Input Detection (live from 5 August 2026) runs a machine-learning model on the client's input stream to flag inputs a human could not produce. | Verified (secondary) | Tech Times, 30 July 2026. The article gives no detection or false-positive rates, and describes a flag as one signal among several. | Closest commercial relative of the input probe. Read FACEIT's own announcement. |

## C. Hardware and security claims

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| C1 | Secrets have been extracted from AMD's firmware TPM. | Verified (secondary) | 'faulTPM' (TU Berlin, arXiv 2304.14717, 2023): voltage fault injection against Zen 2 and Zen 3 processors extracts a chip-unique secret and then the fTPM's stored material. It needs physical access and equipment. AMD acknowledged the report. | Cite the paper. |
| C2 | The bus to discrete TPM chips has been sniffed. | Unverified | From general knowledge. Not searched. | Source or remove. |
| C3 | AMD SEV-SNP and Intel TDX run VMs the host cannot read, only on server CPUs, and consumer GPUs cannot join them. | Unverified | Not searched. | Source or remove; the record already calls this a long-term direction. |
| C4 | Measured-boot attestation is the same technology as DRM and gives studios power to reject custom kernels. | Unverified | General knowledge. | Source before use. |
| C5 | Allowlisted signed kernels (SteamOS, distro kernels) solve the untrusted-kernel problem for attestation. | Unverified | A design claim. Attestation is optional and not built. | Treat as a design idea only. |

## D. Design claims about Rearguard

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| D1 | Humans aim closed-loop, so their errors do not follow the secret drift. | Own measurement (simulation) | In simulation the human model's flick error carries a small share of the drift (slope about 0.26 after 25,692 shots in task 1.2), so the claim is only approximately true even there. No real player data has been through the detector. | Do not state as fact. Real sessions (task 1.10) decide it. |
| D2 | A drift of about 0.5% is below what a player can feel. | Not yet measured | The blind test (task 1.11) has not run. One developer playtest scored 6 of 16 drifted comparisons correct, which is chance but is one person who knows the design. | Do not claim until the blind test has data. |
| D3 | Open-loop cheats (macros, computed flicks, humanised and adaptive aimbots) follow the drift and can be told apart from humans. | Own measurement (simulation) | docs/eval-1.12.md, docs/detector-1.3a.md: at 0.5% drift, 60-second sessions, 0.1% target, the flick, adaptive and fast adaptive aimbots are flagged 100% (500 of 500), the humanised aimbot 11.2% and the recoil macro 15.4%. With 15 minutes of spraying the recoil macro reaches 99.8% scored once and 84.8% under continuous monitoring. | Always say simulated, and always give the setting. |
| D4 | A closed-loop smoothing aimbot evades the fire-time correlation. | Own measurement (simulation) | Invisible at every smoothing setting tried in task 1.2a. A step-size statistic flagged the simulated one 74.4% at 60 seconds, but the simulated cheat has no noise. | State the limit. A real noisy cheat may evade the step statistic too. |
| D5 | The false-positive rate is 0.1%. | Own measurement (simulation) | A threshold calibrated to 0.1% on simulated humans. Held out: 0.12% [0.06, 0.26] for flick and 0.04% [0.01, 0.15] for spray. A study of tens of people can resolve only about 5 to 10%. | Never present as a real-world rate. The live pilot (task 3.9) has to resolve it. |
| D6 | About 200 matches of evidence justify an automatic hardware ban. | Unverified | An estimate from the discussion, not derived from data. | Replace with a derived threshold (decision D8). |
| D7 | Evidence is near-binary, so bans can be automated without a review team. | Unverified | Depends on real false-positive rates, which are unknown. | Soften until real data exists. |
| D8 | Rearguard runs natively on Linux and under Proton. | Partly verified | Linux and Windows are tested in CI (Rust and Godot headless tests). Proton and the Steam Deck have not been tested. | Say 'Linux and Windows' until Proton is tested (task 3.4). |
| D9 | Integration effort is comparable to the EAC SDK. | Unverified | Integration into a real game has not been done or timed. | Measure in task 3.8. |
| D10 | Rearguard is cheap to run. | Partly verified | Client cost measured on headless bots: worst-case frame cost about 245 microseconds at p99 with the uplink, and 208 to 460 KiB a minute of telemetry (up to about 2.3 MiB a minute for a continuously moving 1000 Hz mouse). Server cost per player has not been benchmarked. | Benchmark in task 3.2 before claiming. |
| D11 | On native Wayland Godot 4.7.2 reports accelerated mouse deltas, not raw counts. | Partly verified | From reading the engine source in task 1.4. A hands-on X11 versus Wayland check has not been reported. | Measure in task 3.4. |
| D12 | Microscope mode, split truth, decoys, culling and adversarial textures work as designed. | Not yet measured | None of these is built. The records describe design intent. | Do not describe them as working. |

## E. Novelty claims

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| E1 | Split truth (drawn head versus data head) is new. | Unverified | Not found in the research done so far. A negative claim from limited searching. | Phrase as 'not found in our search', or search more. |
| E2 | The input probe (a secret drift with correlation) is new. | Unverified | Not found in the research. FACEIT's Human Input Detection classifies input passively with a trained model, which is a different approach. | Phrase as 'not found in our search'. |
| E3 | Combining all three probe stages in one OS-agnostic drop-in SDK has not been done. | Unverified | Only the input stage exists in Rearguard so far. | Do not claim until more than one stage is built. |

## F. Competitor figures used on the results page

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| F1 | Riot League of Legends: time to action fell from over 45 games to under 10, and fewer than 1 in 10,000 bans were reversed. | Verified (primary) | Riot's Vanguard x LoL retrospective. The reversal figure divides by bans issued, not by innocent players. | Do not compare it with a per-player false-positive rate. |
| F2 | About 1 in 200 ranked League games had a scripter, and over 175,000 accounts were banned. | Verified (secondary) | esports.net reporting a Riot update. | Find the Riot post. |
| F3 | VALORANT: over 100,000 accounts banned between 22 December 2024 and 13 January 2025. | Verified (secondary) | Strafe, from a graph posted by Riot's head of anti-cheat. | Find the original post. |
| F4 | RICOCHET: over 228,000 bans since Black Ops 6, 23% of cheaters removed before their first match, new cheating accounts banned within 4 matches on average, over 293,000 accounts year to date through June. | Verified (primary) | callofduty.com blog posts and an official Season 5 Q&A. Ban counts, not rates. | Cite the posts. |
| F5 | Activision acknowledged in October 2024 that a detection workaround affected a small number of legitimate accounts. | Verified (secondary) | Dexerto reporting an Activision statement. | Find the statement. |
| F6 | FACEIT early figures: cheaters played 33.4 matches on average before a ban and 58% were banned by their fifth game. | Verified (secondary) | Fragbite, for FACEIT's early client anti-cheat. | Old figure. Do not present it as current. |
| F7 | Academic detectors: AimDetect reports a detection rate of 90 to 92% with false positives from 0.3 to 5.6% in a review, while the paper's abstract says as low as 0.7%. A 2026 preprint reports 88.6% accuracy at a 0.97% false-positive rate for an LSTM and 96.2% at 2.68% for a decision tree. | Partly verified | Paper, SPIE review and arXiv 2607.04336, all on the authors' own datasets. The AimDetect figures disagree between the review and the abstract. | Read the papers before quoting any number. |

## G. Our own measurements and where they live

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| G1 | Probe generator cost: key derivation about 999 ns, one drift sample 10 to 94 ns. | Own measurement (benchmark) | Task 1.1 criterion benchmark on one machine. | Quote with the hardware. |
| G2 | Probe maths is identical on Linux and Windows. | Own measurement | Golden vectors pass in CI on both, and an independent Python version reproduces them. | Safe to state. |
| G3 | The server keeps secrets out of logs, output and the database. | Own measurement | A scanner test searches for the master secret, seeds and resume tokens on Linux and Windows CI, with a control test. | Safe to state. |
| G4 | Developer playtest of the packaged build. | Own measurement (human) | One person who knows the design: all facilitator checks passed, 46 rounds replayed, X11 raw input. Not usable as study data. | Do not use as evidence of imperceptibility. |

## H. Things not to say

| ID | Claim | Status | Evidence and source | Before use |
|---|---|---|---|---|
| H1 | Rearguard is unbreakable. | Do not claim | Nothing is. The concept record and README both say so. | Never. |
| H2 | Rearguard covers all kinds of cheats. | Do not claim | It is the aim. Only the input layer exists. | Say 'aims to'. |
| H3 | Detection or false-positive rates, as real-world rates. | Do not claim | Every number is simulated. | Say 'simulated'. |
| H4 | Rearguard will make an industry standard or grow a platform's market share. | Do not claim | A hope, not a result. | Keep it as a hope. |

## Sources

Found by search during the 1 October 2026 check. Read each in full before it goes into a pitch.

- Riot, Vanguard x LoL retrospective: https://www.leagueoflegends.com/en-us/news/dev/dev-vanguard-x-lol-retrospective/
- Riot, Demolishing Wallhacks with VALORANT's Fog of War: https://www.riotgames.com/en/news/demolishing-wallhacks-valorants-fog-war
- Activision, Ricochet Season 04 update (Hallucinations): https://www.callofduty.com/blog/2023/06/call-of-duty-ricochet-anti-cheat-season-04-update
- Activision, Ricochet Season 3 recap: https://www.callofduty.com/blog/2025/05/call-of-duty-black-ops-6-warzone-ricochet-anti-cheat-season-three-recap
- Invisibility Cloak, USENIX Security 2024: https://www.usenix.org/conference/usenixsecurity24/presentation/sun-chenxin
- AimTrap, arXiv 2606.25734: https://arxiv.org/pdf/2606.25734
- Aim Low, Shoot High (Witschel and Wressnegger, 2020): https://arxiv.org/pdf/2004.12183
- faulTPM, arXiv 2304.14717: https://arxiv.org/abs/2304.14717
- Epic and BattlEye Linux and Proton support (PC Gamer): https://pcgamer.com/battleye-anti-cheat-confirms-steam-deck-support
- BattlEye Proton support (Phoronix): https://www.phoronix.com/news/BattlEye-Proton-Steam-Deck
- FACEIT Human Input Detection (Tech Times): https://www.techtimes.com/articles/322336/20260730/faceit-human-input-detection-catching-cheats-that-signature-scans-miss.htm
- Valve Trust Factor (Destructoid): https://vip-develop.destructoid.com/?p=208262
- AimDetect, Liu et al., DSN 2017: https://www.eecis.udel.edu/~hnw/paper/dsn17a.pdf
- 2026 preprint on server-side aimbot detection: https://arxiv.org/html/2607.04336v1
- CornerCulling: https://github.com/87andrewh/CornerCulling
