# Provenance and publication

The repository starts with a new root commit. No earlier Git objects, runtime code, binaries, third-party file schemas, research, working configurations, measurements or development logs are imported.

Architecture and roadmap are written for KERN from owner requirements. The three-layer SVG, README illustrations and color tokens are approved project-owned artwork. Only independently authored MIT context/translation tools and their tests are carried over; their original copyright notice remains in LICENSE. This is a verifiable transfer boundary, not legal clearance for any future material.

New code is MIT; third-party dependencies need separate license/provenance review. Do not add external code, resources, specifications or compatible formats without recorded sources/grounds. Renaming or packaging cannot bypass the publication boundary. Synthetic tests need their own generator.

Conversational comparisons with third-party interfaces become independent KERN requirements: readability, information density, behavior and resource budgets. Product names, links and nickname mappings are not published, even when mentioned in conversation. If context is necessary, a neutral description such as “a program with a white interface” is acceptable.

Before publication, review the diff, index and entire new history; exclude local paths, credentials and real data. The logo/palette define style; the README illustration is not a running product screenshot.

For im2480, a new Modbus RTU reader and independent synthetic peer were written, with public primary transport references in the [contract](IM2480.md). A restricted profile of the owner's emulator is used; earlier application sources/binaries and private research are not included. Previously selected port settings were restored only in the local bundle configuration without copying the old profile.

Approved prototype dependencies redb/uPlot come from official Cargo/npm distributions, pinned with licenses and artifact verification: [DEPENDENCIES](DEPENDENCIES.md). The interpreter, open CSV and backup JSONL were written for KERN; previous implementations/formats were not transferred.
