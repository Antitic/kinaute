# Origine des formats, polices et ressources

## Formats de fichier

- **`.pptx`** : Office Open XML (ECMA-376 / ISO/IEC 29500), norme publique. Kinaute lit et écrit PresentationML et DrawingML d'après cette
  norme et d'après l'observation de fichiers de test. Aucun code ni schéma d'un tiers n'est repris.
- **`.key` (Keynote)** : **non pris en charge** (décision du commanditaire). Aucune définition de format Apple n'est utilisée.

## Marques

- « Keynote » et « Apple » sont des marques d'Apple Inc. ; « PowerPoint » est une marque de Microsoft. Kinaute n'est affilié à aucune des deux.
- **Risque connu** : le nom « Kinaute » est phonétiquement proche de « Keynote ». Voir `docs/decisions.md` (décision 5).
- L'interface reprend l'organisation habituelle des logiciels de présentation (navigateur à gauche, inspecteur à droite, barre d'outils)
  mais **aucune icône, image, couleur ni police d'Apple ou de Microsoft n'est utilisée**.

## Polices

Kinaute **n'embarque aucune police**. Il utilise les polices du système. Quand une police d'Office est absente, il choisit un équivalent libre
de mêmes métriques s'il est installé :

| Demandée | Remplacée par |
|---|---|
| Calibri, Segoe UI | Carlito (OFL), puis Inter, DejaVu Sans |
| Cambria | Caladea (OFL), puis DejaVu Serif |
| Arial, Helvetica | Liberation Sans (OFL), Arimo, FreeSans |
| Times New Roman | Liberation Serif (OFL), Tinos, FreeSerif |
| Courier New | Liberation Mono (OFL), Cousine, FreeMono |

Un nom de police absent sans équivalent est remplacé par la famille générique (sans-serif / serif / monospace) ; le fichier n'est pas modifié.

## Thème et modèle de document

Le thème « Kinaute » (palette, polices Arial, styles) et les trois dispositions du document vierge (`crates/pptx/template/`) sont **originaux**
et placés sous GPL-3.0-or-later comme le reste du projet.

## Fichiers de test

Les corpus servant aux tests (python-pptx — MIT ; Apache POI — Apache-2.0) sont **téléchargés localement et non redistribués** dans ce dépôt
(`/corpus-dl` est ignoré par git). Aucun fichier de test n'est versionné tant que sa licence n'a pas été vérifiée.
Le comparateur de rendu utilise LibreOffice (MPL-2.0) comme simple outil de régression, sans reprise de code.

## Dépendances

Principales : `slint` (GPL-3.0 ou licence commerciale ; utilisé sous GPL-3.0), `tiny-skia` (BSD-3), `cosmic-text` (MIT/Apache-2.0),
`quick-xml` (MIT), `zip` (MIT), `image` (MIT/Apache-2.0), `thiserror`, `tracing`. La liste complète et à jour : `cargo tree`.
