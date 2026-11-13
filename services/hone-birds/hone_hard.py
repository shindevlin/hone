"""Hard birds: group-level sightings, photo sightings and upgrades, broadcast holds (hone-birds indexer).

Answers Birderlands' proposal `hone-proposal-hard-birds` (2026-11-12). Josh asked Hone to apply it with
Claude's defaults (2026-11-13); every number marked OURS can be tuned.

Levels. A sighting entry may carry "level": "species" (default) | "genus" | "group".
  genus  {"level":"genus","name":"Anas","conf_q8":..}  stored as latin "Anas sp."
  group  {"level":"group","name":"ducks","conf_q8":..} stored as latin "Anatidae sp." (eBird-style "sp." names)
  conf_q8 is the summed score of the genus/group (the phone sums it; Birderlands' groupGuess does).
  Accepted when it clears the same bar as a species AND at least one species of that genus/group is
  recorded in the region (GBIF range). Out of region: held back like an unlikely species.

Broadcast and playback (none is decisive alone; all hold, none rejects):
  "fp_match": true      the phone matched a known reference recording   -> held (2), never public
  "playback_q8": 0-255  the phone's speaker-signature score; >= PLAYBACK_HOLD_Q8 -> held for review (5)
  "t": unix seconds of the first frame; enables the crowd check: the same taxon from >= CROWD_DEVICES
       accounts in >= CROWD_CELLS res-7 cells, two of them > CROWD_KM apart, within +-CROWD_S, AND a sign
       it isn't just a common bird singing everywhere (out of range in one of the cells, or a playback
       score on two reports) -> all of them held for review (5).
  Held rows are not public and earn nothing until released: an in-app photo with good evidence from the
  same account, or an admin release (POST /api/birds/admin/release {"sid"}).

Photos. An in-app photo with good evidence and a "cell" (res <= 9) is a sighting of its own
  (source "photo"). Within UPGRADE_WINDOWS (30 min) and the same or a neighbouring res-9 cell, it upgrades
  the same account's genus/group record that contains the species: a species row is added with the
  original time and cell (upgraded_from = the group row), and the group row is marked upgraded (6).

Earning is not decided here: the chain pays (usefulness rules, behind an activation epoch). The
`earn_class` column tells it what kind of record this is: species | genus | group | photo | upgrade.
"""
import math
import re

import h3

# OURS: tune freely.
PLAYBACK_HOLD_Q8 = 179      # 0.7
CROWD_DEVICES, CROWD_CELLS, CROWD_KM, CROWD_S = 5, 3, 20.0, 60
CROWD_PLAYBACK_Q8 = 102     # 0.4: "some playback signal" for the crowd check's second condition
UPGRADE_WINDOWS = 2         # 15-min windows either side (about 30 min)
GENUS = re.compile(r"^[A-Z][a-z]+$")

# verified states used by the indexer: 0 unbound seat, 1 public, 2 held (never public), 3 rare unconfirmed,
# 4 rare confirmed, 5 held for review (possible broadcast/playback), 6 upgraded (superseded by a species row)
HELD_REVIEW, UPGRADED = 5, 6

# Same groups as Birderlands engine/Groups.kt, by genera; the public name uses the family/order "sp." form.
GROUPS = {
    "ducks": ("Anatidae sp.", """Anas Spatula Mareca Sibirionetta Aythya Netta Dendrocygna Cairina Sarkidiornis Oxyura Nomonyx Mergus
        Lophodytes Mergellus Bucephala Melanitta Somateria Polysticta Clangula Histrionicus Aix Branta Anser Cygnus Tadorna Chloephaga
        Amazonetta Callonetta Nettapus Merganetta Speculanas Lophonetta Tachyeres Plectropterus Alopochen Stictonetta Malacorhynchus
        Asarcornis Coscoroba Cereopsis Heteronetta Marmaronetta Salvadorina Hymenolaimus Pteronetta Thalassornis Biziura"""),
    "parrots": ("Psittaciformes sp.", """Amazona Pionus Psittacara Eupsittula Aratinga Ara Primolius Orthopsittaca Diopsittaca
        Anodorhynchus Cyanopsitta Guaruba Brotogeris Bolborhynchus Pyrrhura Forpus Pyrilia Pionopsitta Pionites Deroptyus Graydidascalus
        Alipiopsitta Hapalopsittaca Touit Nannopsittaca Psilopsiagon Myiopsitta Cyanoliseus Rhynchopsitta Ognorhynchus Leptosittaca
        Enicognathus Thectocercus Nandayus Psittacula Psittacus Poicephalus Agapornis Cacatua Calyptorhynchus Zanda Eolophus Nymphicus
        Platycercus Melopsittacus Trichoglossus Eos Lorius Loriculus Nestor Strigops Eclectus Tanygnathus Prioniturus Psittaculirostris
        Cyclopsitta Polytelis Aprosmictus Alisterus Psephotus Northiella Neophema Pezoporus Barnardius Lathamus Cyanoramphus Coracopsis
        Psittrichas Probosciger Lophochroa Callocephalon Glossopsitta Parvipsitta Neopsittacus Charmosyna Chalcopsitta Pseudeos Vini
        Psittinus Micropsitta Geoffroyus"""),
    "gulls": ("Laridae sp.", """Larus Leucophaeus Chroicocephalus Ichthyaetus Hydrocoloeus Rissa Pagophila Rhodostethia Xema Creagrus
        Sterna Thalasseus Sternula Gelochelidon Hydroprogne Chlidonias Onychoprion Anous Procelsterna Gygis Larosterna Phaetusa Rynchops"""),
    "hummingbirds": ("Trochilidae sp.", """Amazilia Saucerottia Amazilis Archilochus Calypte Selasphorus Cynanthus Chlorostilbon
        Riccordia Campylopterus Pampa Eugenes Lampornis Heliomaster Colibri Anthracothorax Phaethornis Heliodoxa Florisuga Chlorestes
        Hylocharis Basilinna Lophornis Thalurania Chalybura Eupherusa Atthis Doricha Calothorax Tilmatura Abeillia Klais Glaucis
        Threnetes Eupetomena Coeligena Aglaiocercus Heliangelus Boissonneaua Ocreatus Metallura Oreotrochilus Patagona Ensifera Lesbia
        Chaetocercus Mellisuga Leucolia Uranomitra Chrysuronia Polytmus Topaza Discosura Heliothryx Augastes Adelomyia Aphantochroa
        Sternoclyta Lepidopyga Goldmania Elvira Microchera Panterpe Lamprolaima Orthorhyncus Eulampis Trochilus Calliphlox Myrtis
        Rhodopis Thaumastura Eulidia Myrmia Hylonympha Juliamyia Leucippus Taphrospilus Talaphorus"""),
    "hawks": ("Accipitriformes sp.", """Accipiter Astur Buteo Buteogallus Rupornis Parabuteo Geranoaetus Geranospiza Harpagus Ictinia
        Elanus Elanoides Chondrohierax Leptodon Rostrhamus Helicolestes Circus Haliaeetus Aquila Hieraaetus Spizaetus Spizastur Harpia
        Morphnus Pseudastur Leucopternis Busarellus Milvus Pernis Haliastur Circaetus Kaupifalco Butastur Melierax Micronisus
        Polyboroides Terathopius Stephanoaetus Polemaetus Lophaetus Nisaetus Ictinaetus Clanga Pandion"""),
    "owls": ("Strigiformes sp.", """Strix Ciccaba Bubo Megascops Otus Glaucidium Athene Aegolius Asio Pseudoscops Pulsatrix Lophostrix
        Micrathene Psiloscops Tyto Ninox Surnia Ketupa Scotopelia Phodilus Taenioptynx Heteroglaux Xenoglaux Margarobyas Gymnasio"""),
    "herons": ("Ardeidae sp.", """Ardea Egretta Bubulcus Butorides Nycticorax Nyctanassa Ixobrychus Botaurus Tigrisoma Cochlearius
        Pilherodius Agamia Syrigma Zebrilus Ardeola Gorsachius Zonerodius Dupetor"""),
    "doves": ("Columbidae sp.", """Columba Patagioenas Streptopelia Zenaida Columbina Leptotila Geotrygon Zentrygon Claravis
        Paraclaravis Metriopelia Uropelia Geopelia Ocyphaps Spilopelia Treron Ducula Ptilinopus Chalcophaps Macropygia Turtur Oena
        Caloenas Goura Phaps Starnoenas Gallicolumba Alopecoenas Leucosarcia Geophaps Hemiphaga Reinwardtoena"""),
    "woodpeckers": ("Picidae sp.", """Melanerpes Picoides Dryobates Leuconotopicus Dryocopus Campephilus Colaptes Sphyrapicus Celeus
        Veniliornis Picumnus Piculus Dendrocopos Dendrocoptes Picus Dinopium Jynx Chrysocolaptes Meiglyptes Mulleripicus Micropternus
        Hemicircus Sasia Nesoctites Geocolaptes Campethera Dendropicos Chloropicus Yungipicus Blythipicus Gecinulus Reinwardtipicus
        Chrysophlegma"""),
    "flycatchers": ("Tyrannidae sp.", """Empidonax Contopus Myiarchus Tyrannus Pitangus Megarynchus Myiozetetes Myiodynastes Legatus
        Sayornis Pyrocephalus Camptostoma Elaenia Mionectes Todirostrum Tolmomyias Myiopagis Zimmerius Attila Mitrephanes Xenotriccus
        Myiobius Terenotriccus Oncostoma Poecilotriccus Rhynchocyclus Platyrinchus Lophotriccus Phyllomyias Ornithion Capsiempis
        Serpophaga Tyrannulus Leptopogon Phylloscartes Fluvicola Arundinicola Colonia Machetornis Conopias Philohydor Satrapa Knipolegus
        Hymenops Agriornis Muscisaxicola Xolmis Ochthoeca Silvicultrix Lathrotriccus Cnemotriccus Myiophobus Pyrrhomyias Hirundinea
        Sublegatus Inezia Euscarthmus Stigmatura Anairetes Mecocerculus Hemitriccus Myiornis Atalotriccus Empidonomus Griseotyrannus
        Myiotheretes Sirystes Rhytipterna Casiornis Ramphotrigon Deltarhynchus Onychorhynchus Pogonotriccus"""),
}
GROUPS = {k: (name, frozenset(genera.split())) for k, (name, genera) in GROUPS.items()}
GROUP_BY_NAME = {name: k for k, (name, _) in GROUPS.items()}
SCHEMA_COLS = ("level TEXT", "taxon TEXT", "t_first INTEGER", "playback_q8 INTEGER", "fp_match INTEGER",
               "flock INTEGER", "upgraded_from TEXT", "hold TEXT", "earn_class TEXT")


def migrate(db, sqlite3):
    for col in SCHEMA_COLS:
        try:
            db.execute("ALTER TABLE sightings ADD COLUMN " + col)
        except sqlite3.OperationalError:
            pass  # already there
    db.execute("CREATE INDEX IF NOT EXISTS s_tfirst ON sightings(t_first)")


def parse(entry, latin_re):
    """-> (level, taxon, stored latin, genera) for one species entry, or None if malformed."""
    level = entry.get("level") or "species"
    name = str(entry.get("name", ""))
    if level == "species":
        return ("species", name, name, frozenset([name.split(" ")[0]])) if latin_re.match(name) else None
    if level == "genus":
        return ("genus", name, name + " sp.", frozenset([name])) if GENUS.match(name) else None
    if level == "group":
        g = GROUPS.get(name.lower())
        return ("group", name.lower(), g[0], g[1]) if g else None
    return None


def genera_of_latin(latin):
    """Genera a stored latin can stand for: 'Anas acuta' -> {Anas}, 'Anas sp.' -> {Anas}, 'Anatidae sp.' -> ducks."""
    if latin in GROUP_BY_NAME:
        return GROUPS[GROUP_BY_NAME[latin]][1]
    return frozenset([latin.split(" ")[0]])


def taxon_in_range(names, genera):
    """True if any species recorded in the region belongs to these genera; None if the range isn't known."""
    if names is None:
        return None
    return any(n.split(" ")[0] in genera for n in names)


def km(c1, c2):
    a, b = h3.cell_to_latlng(c1), h3.cell_to_latlng(c2)
    p1, p2, dl = math.radians(a[0]), math.radians(b[0]), math.radians(b[1] - a[1])
    x = math.sin((p2 - p1) / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 6371.0 * 2 * math.asin(math.sqrt(min(1.0, x)))


def to_res(cell, res):
    r = h3.get_resolution(cell)
    return h3.cell_to_parent(cell, res) if r > res else cell


def crowd_check(db, latin, t, in_range_fn):
    """Same taxon at the same minute from many accounts in far-apart places = probably a broadcast.
    Returns the sids to hold, or []."""
    rows = db.execute("SELECT sid, cell, account, playback_q8 FROM sightings WHERE latin=? AND t_first BETWEEN ? AND ? "
                      "AND verified IN (1,3,4,5)", (latin, t - CROWD_S, t + CROWD_S)).fetchall()
    accounts = {r[2] for r in rows}
    if len(accounts) < CROWD_DEVICES:
        return []
    cells7 = sorted({to_res(r[1], 7) for r in rows})
    if len(cells7) < CROWD_CELLS:
        return []
    if not any(km(a, b) > CROWD_KM for i, a in enumerate(cells7) for b in cells7[i + 1:]):
        return []
    # Second condition: not just a common bird singing in many places at dawn.
    odd_place = any(in_range_fn(c, latin) is False for c in cells7)
    playback = sum(1 for r in rows if (r[3] or 0) >= CROWD_PLAYBACK_Q8) >= 2
    return [r[0] for r in rows] if (odd_place or playback) else []


def photo_sighting(db, photo_id, latin, account, pub, t_ms, cell, show_name, in_range_fn, now):
    """An in-app photo with good evidence and a cell becomes a sighting, and upgrades the same account's
    nearby genus/group record that contains the species. Returns (sighting rows added, upgraded sids)."""
    if not cell or not h3.is_valid_cell(cell) or h3.get_resolution(cell) > 9:
        return 0, []
    w = int(t_ms) // 900_000
    res = h3.get_resolution(cell)
    rng = in_range_fn(cell, latin)
    v = 3 if rng is False else 1
    added = db.execute("INSERT OR IGNORE INTO sightings(sid,window,cell,res,latin,conf_q8,frames,model,account,pub,received,"
                       "verified,show_name,source,level,taxon,earn_class) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                       (f"photo:{photo_id}", w, cell, res, latin, 255, 0, "photo", account, pub, now, v, show_name,
                        "photo", "species", latin, "photo")).rowcount
    genus = latin.split(" ")[0]
    near = set(h3.grid_disk(to_res(cell, 9), 1))
    upgraded = []
    for sid, gcell, gwin, glatin, gv, gshow in db.execute(
            "SELECT sid, cell, window, latin, verified, show_name FROM sightings WHERE account=? AND level IN ('genus','group') "
            "AND verified IN (1,2,3,4,5) AND ABS(window-?)<=?", (account, w, UPGRADE_WINDOWS)).fetchall():
        if genus not in genera_of_latin(glatin):
            continue
        if to_res(gcell, 9) not in near and not (h3.get_resolution(gcell) < 9 and to_res(cell, h3.get_resolution(gcell)) == gcell):
            continue
        db.execute("INSERT OR IGNORE INTO sightings(sid,window,cell,res,latin,conf_q8,frames,model,account,pub,received,"
                   "verified,show_name,source,level,taxon,upgraded_from,earn_class) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                   (sid + ":up", gwin, gcell, h3.get_resolution(gcell), latin, 255, 0, "photo-upgrade", account, pub, now,
                    3 if rng is False else 1, gshow, "upgrade", "species", latin, sid, "upgrade"))
        db.execute("UPDATE sightings SET verified=?, hold='upgraded' WHERE sid=?", (UPGRADED, sid))
        upgraded.append(sid)
    return added, upgraded
