# ADR 0001 — `.pptx` est le format natif

**Statut** : accepté.

**Contexte** : l'objectif est de travailler avec des utilisateurs de PowerPoint sans friction. Le plan initial prévoyait un format natif
(`.key`) plus des convertisseurs `.pptx`.

**Décision** : Kinaute ouvre et enregistre `.pptx` directement ; le modèle de document est une vue typée de PresentationML/DrawingML.

**Alternatives écartées** :
- *Format natif propre + import/export `.pptx`* : chaque aller-retour avec un collègue perd de l'information.
- *`.key`* : format non documenté, rétro-ingénierie, risque que Keynote refuse les fichiers écrits ; abandonné par le commanditaire.

**Conséquences** : seules les fonctions représentables en `.pptx` sont proposées. Le modèle suit les unités de DrawingML (EMU, 60000ᵉ de degré…).
Le rendu doit reproduire celui de PowerPoint, pas celui de Keynote.
