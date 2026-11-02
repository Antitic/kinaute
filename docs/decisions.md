# Décisions

Questions posées au commanditaire une à la fois, avec sa réponse. Le plan initial a été remis en cause point par point ; ce
qui suit remplace ce plan là où il diffère.

| # | Question | Décision | Conséquence |
|---|---|---|---|
| 1 | Problème à résoudre | Une application à l'interface très proche de celle de Keynote, en Rust, légère, pour travailler avec des utilisateurs de PowerPoint. Toutes les fonctions de Keynote… | …dans la limite de ce que `.pptx` sait représenter (voir 3) |
| 2 | Format `.key` | **Abandonné.** | Plus de rétro-ingénierie, plus de risque juridique sur le format, `LEGAL.md` réduit aux polices et assets |
| 3 | Format de travail | **`.pptx` est le seul format**, sans conversion et sans risque de perte. On ne propose que des fonctions que PowerPoint a. | Voir ADR 0001 et 0004 |
| 4 | Plateforme | **Linux d'abord**, architecture portable (Windows, macOS ensuite). | IME, accessibilité, polices système et empaquetage restent à faire par plateforme |
| 5 | Nom | **Kinaute** | Risque connu : ce nom se prononce presque comme « Keynote » (confusion de marque possible, surtout avec une interface volontairement proche). À réévaluer avant toute diffusion large. Alternatives proposées : Lanterne, Estrade, Diorama |
| 6 | Licence | **GPL-3.0-or-later** pour tout le projet | Les dépendances doivent être compatibles GPL-3 |
| 7 | « Méga légère » | Démarrage instantané, très peu de mémoire, fluidité extrême, très vieilles machines. Taille du binaire : sans importance | Voir ADR 0002 : le rendu CPU est le moteur principal |
| 8 | Machine de référence | **PC de 2008** (Core 2 Duo, 2 Go, OpenGL 2.1) | Écarte GTK4 et tout ce qui exige un GPU récent |
| 9 | Première version | « Un premier jet très fonctionnel, testé au fur et à mesure » | Lecture/rendu/écriture solides d'abord, puis édition |

## Ce que le plan initial prévoyait et qui change

- **GPU principal / CPU en repli** → **CPU principal** ; GPU éventuellement plus tard, comme accélération facultative.
- **`.key` natif** → supprimé. **Export `.pptx`** → c'est le format natif.
- **Workspace de 11 crates** → 6 crates (voir README). `layout`, `commands`, `anim`, `io-export`, `present`, `testkit` ne justifiaient pas
  encore une crate séparée ; leur rôle est tenu par `model`, `render` et `app`. À séparer quand le code le demandera.
- **Benchmark vello/lyon/skia avant de choisir** → non fait : les contraintes (aucun GPU, vieilles machines) imposaient déjà un rendu
  logiciel ; tiny-skia a été retenu et mesuré (voir `docs/status.md`).
- **Prototype comparatif des frameworks d'interface (une semaine)** → non fait comme tel. Slint a été retenu sur ses propriétés
  (rendu logiciel natif, licence GPL-3 compatible, accessibilité AccessKit) et **mesuré** (démarrage, mémoire, image par image).
  Si ces mesures se dégradaient, c'est la première décision à rouvrir.

## Questions encore ouvertes

1. Les fonctions propres à Keynote sans équivalent PowerPoint sont exclues : faut-il tout de même les exposer avec un avertissement à l'export ?
2. Faut-il viser la compatibilité visuelle « pixel-perfect » avec PowerPoint ? Aujourd'hui le rendu est comparé à LibreOffice (régressions
   seulement), faute de PowerPoint dans l'environnement de test.
3. Polices : embarquer des polices libres (Carlito, Liberation — OFL) dans le binaire, ou dépendre des polices système ?
