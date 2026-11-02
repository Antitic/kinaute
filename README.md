# Kinaute

Éditeur de présentations **léger et rapide**, écrit en Rust, qui travaille **directement sur des fichiers `.pptx`** :
on ouvre le fichier d'un collègue sous PowerPoint, on le modifie, on l'enregistre — sans conversion et sans perte.

> Statut : premier jet fonctionnel (v0.1). Voir [`docs/status.md`](docs/status.md) pour ce qui marche, ce qui manque et les écarts avec le plan initial.

## Principes

- **`.pptx` est le seul format.** Pas de format natif intermédiaire, pas de `.key`. Les fonctions proposées sont celles que PowerPoint sait représenter.
- **Zéro perte.** Tout ce que Kinaute ne comprend pas (graphiques, SmartArt, animations d'objets, polices incorporées, extensions…) est conservé tel quel à l'enregistrement.
- **Léger.** Rendu logiciel (CPU) : aucune carte graphique requise. Cible : un PC de 2008 (Core 2 Duo, 2 Go, OpenGL 2.1).
- **Linux d'abord**, architecture portable (Windows, macOS ensuite).

## Mesures (un seul cœur, Linux, binaire release)

| | Résultat |
|---|---|
| Fenêtre visible, document vierge | 66 ms |
| Fenêtre visible, 48 diapositives avec images | 143 ms |
| Fenêtre visible, fichier de 2,2 Mo | 227 ms |
| Mémoire après 5 s (vierge / 48 diapo. / 2,2 Mo) | 37 / 75 / 54 Mo |
| Image pendant un glisser (édition + rendu complet) | 11 à 19 ms |
| Taille du binaire | 21 Mo |

Reproduire : `scripts/gui-env.sh start`, puis voir `docs/status.md` (section « Mesures »).

## Construire et lancer

```bash
cargo build --release
./target/release/kinaute [fichier.pptx]
```

Polices : Kinaute utilise les polices du système. Les polices courantes d'Office sont remplacées par des équivalents métriques libres
quand elles existent (Calibri → Carlito, Arial → Liberation Sans…). Voir [`LEGAL.md`](LEGAL.md).

## Utilisation

| Action | Raccourci |
|---|---|
| Nouveau / Ouvrir / Enregistrer / Enregistrer sous | Ctrl+N / Ctrl+O / Ctrl+S / Ctrl+Maj+S |
| Annuler / Rétablir | Ctrl+Z / Ctrl+Maj+Z (ou Ctrl+Y) |
| Copier / Couper / Coller / Dupliquer | Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+D |
| Tout sélectionner | Ctrl+A |
| Grouper / Dissocier | Ctrl+G / Ctrl+Maj+G |
| Nouvelle diapositive | Ctrl+M |
| Gras / Italique / Souligné | Ctrl+B / Ctrl+I / Ctrl+U |
| Déplacer l'objet (1 pt, 10 pt avec Maj) | Flèches |
| Supprimer | Suppr / Retour arrière |
| Diaporama (depuis le début / la sélection) | F5 / Maj+F5 |
| Zoom | Ctrl+molette, Ctrl+`+`, Ctrl+`-`, Ctrl+0 |
| Diapositive précédente / suivante | Page haut / Page bas |

Souris : glisser pour déplacer, poignées pour redimensionner (Maj garde les proportions), poignée ronde pour pivoter (Maj = pas de 15°),
guides d'alignement jaunes automatiques, rectangle de sélection sur le fond. Glisser une vignette pour réordonner les diapositives.

Diaporama : flèches, espace, Entrée, clic pour avancer ; Échap pour quitter.
Transitions : fondu, poussée, recouvrement, dévoilement, balayage, fractionner, dissolution, zoom, coupure.

Récupération : un fichier de secours est écrit toutes les 30 s dans `~/.cache/kinaute/recovery/` tant que le document est modifié.

## Outil en ligne de commande

```bash
kinaute-tool info f.pptx            # résumé et rapport d'import
kinaute-tool check *.pptx           # aller-retour ouverture → enregistrement → réouverture
kinaute-tool render f.pptx dir 960  # une image PNG par diapositive
kinaute-tool pdf f.pptx out.pdf     # export PDF (pages raster)
kinaute-tool new out.pptx           # document vierge
kinaute-tool fuzz 25 *.pptx         # test de robustesse sur fichiers corrompus
```

## Architecture

| Crate | Rôle |
|---|---|
| `kinaute-xml` | Arbre XML sans perte (l'ordre, les préfixes et les attributs sont conservés) |
| `kinaute-model` | Modèle de document, héritage (masque → disposition → diapositive), commandes, annuler/rétablir. Aucune dépendance graphique |
| `kinaute-pptx` | Lecture / écriture `.pptx` : un objet inchangé est réécrit à partir de son XML d'origine, les parties inconnues sont copiées octet pour octet |
| `kinaute-render` | Rendu CPU (tiny-skia + cosmic-text), transitions, export PDF |
| `kinaute` | Application (Slint, rendu logiciel) et mode diaporama |
| `kinaute-tool` | Outil en ligne de commande |

Toute modification du document passe par une commande (`kinaute_model::Command`) : c'est ce qui rend l'annulation illimitée possible.

## Licence

GPL-3.0-or-later, voir [`LICENSE`](LICENSE). Décisions et justifications : [`docs/decisions.md`](docs/decisions.md).
