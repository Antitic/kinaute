# État d'avancement (v0.1 — premier jet)

Ce document dit ce qui marche **et comment on l'a vérifié**, ce qui manque, et les écarts avec le plan initial.
Rien n'est annoncé qui n'ait été testé sur des fichiers réels.

## Vérifié

| Fonction | Vérification |
|---|---|
| Lecture `.pptx` | 149 fichiers publics ouvrables sur 161 (python-pptx, Apache POI). Les 12 autres sont des fichiers corrompus volontairement par des fuzzers : rejetés proprement, sans plantage |
| Aller-retour sans perte | `kinaute-tool check` : ouverture → enregistrement → réouverture, modèle identique et parties non modélisées identiques octet pour octet — **149/149** |
| Fichiers modifiés lisibles ailleurs | Document modifié par commandes relu par **python-pptx** et converti par **LibreOffice** (texte, formes, transition, diapositive dupliquée/supprimée/déplacée) |
| Rendu | 161 fichiers rendus, aucun plantage, aucun blocage, aucun fichier au-dessus de 3 s |
| Fidélité visuelle | Comparaison automatique avec LibreOffice (`scripts/compare.py`) : écart moyen **13/255** sur 184 diapositives d'un échantillon de 36 fichiers. LibreOffice est une **référence de non-régression, pas PowerPoint** |
| Robustesse | 3 725 documents dont le XML interne est corrompu (octets, balises, nombres extrêmes, structures dupliquées) : **0 plantage, 0 blocage** ; 95 % de ces documents s'ouvrent encore et sont rendus puis réenregistrés (le test exerce donc bien le code) |
| Éditeur | 5 tests de logique sans fenêtre (créer/déplacer/annuler, redimensionner un objet pivoté, aligner/grouper, saisie + enregistrement) + essais réels sous Xvfb + i3 avec captures d'écran |
| Diaporama | Essayé en plein écran sous Xvfb (fondu en cours capturé, retour à l'éditeur sur la bonne diapositive) |
| Qualité du code | `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` : propres. Pas de `unwrap` hors tests, `unsafe` interdit (sauf code généré par Slint) |

## Mesures (un seul cœur, `taskset -c 0`, binaire release)

| Mesure | Résultat | Cible du plan |
|---|---|---|
| Fenêtre visible, vierge / 48 diapo. / 2,2 Mo | 66 / 143 / 227 ms | < 300 ms |
| Mémoire après 5 s | 37 / 75 / 54 Mo | < 150 Mo |
| Image pendant un glisser | 11–19 ms | 16 ms (60 i/s) |
| Binaire | 21 Mo | sans importance |

Reproduction : `scripts/gui-env.sh start` ; lancer `kinaute` avec `DISPLAY=:99` ; pour l'image par glisser :
`KINAUTE_BENCH_FILE=f.pptx taskset -c 0 cargo test --release -p kinaute -- --ignored --nocapture bench_drag`.
Ces mesures sont faites sur une machine récente bridée à un cœur : elles ne remplacent pas un essai sur un vrai PC de 2008.

## Ce qui fonctionne dans l'éditeur

Création / ouverture / enregistrement (écriture atomique) / enregistrement sous / export PDF (pages raster) ; navigateur de diapositives
(ajouter, dupliquer, supprimer, déplacer, masquer, glisser-déposer) ; texte, formes (10), lignes, images ; sélection (clic, Maj, rectangle),
déplacement, redimensionnement (Maj = proportions), rotation, guides d'alignement automatiques, alignement/répartition, groupement,
ordre d'empilement, copier/couper/coller/dupliquer, flèches ; inspecteur (remplissage, contour, texte, police, gras/italique/souligné,
alignement, position/taille/rotation, transition) ; annuler/rétablir illimité (une saisie ou un glisser = une étape) ; zoom ;
diaporama avec 9 transitions ; récupération automatique ; clair/sombre selon le système.

## Ce qui ne fonctionne pas encore (liste honnête)

**Rendu** (le fichier est conservé intact dans tous les cas)
- **Graphiques** : remplacés par un cadre « Graphique ». C'est le plus gros écart visuel du corpus.
- **Objets incorporés (OLE)** dont l'aperçu est EMF/WMF : cadre « Objet incorporé ».
- Effets d'image : recoloration *duotone*, luminosité/contraste, réflexion, lueur, flou d'objet, 3D, WordArt (texte sur trajectoire).
- Texte vertical, colonnes multiples, texte contour/dégradé, soulignement/barré affichés, couleurs de surlignage, exposant/indice (taille seulement).
- Polices : aucune n'est embarquée ; les polices incorporées au `.pptx` sont conservées mais pas utilisées pour l'affichage.
- Tableaux : styles de tableau intégrés approximés (en-tête + lignes alternées) ; hauteur de ligne non recalculée selon le texte.
- Formes prédéfinies : ~55 dessinées ; les autres sont dessinées en rectangle. Formes libres (`custGeom`) : prises en charge avec leurs formules.

**Édition**
- **Saisie de texte directement sur le canevas** : aujourd'hui dans l'inspecteur (onglet Texte). Pas de sélection partielle de texte (le style
  s'applique à tout l'objet), pas de listes à puces modifiables.
- Tableaux, graphiques, SmartArt, médias audio/vidéo : affichés ou conservés, **non modifiables**.
- Animations d'objets (entrée/sortie/trajectoire), Magic Move/Morph : **conservées** à l'enregistrement mais ni jouées ni modifiables.
  Transitions : 9 types jouables ; les variantes `p14` sont lues, jouées en fondu.
- Masques de diapositives et thèmes : lus et appliqués, **non modifiables** ; une seule disposition par défaut proposée pour « nouvelle diapositive ».
- Notes du présentateur : lues, non affichées ; pas de mode présentateur multi-écrans, ni minuteur.
- Commentaires, impression, export vidéo/images : non faits (export PNG par diapositive possible via `kinaute-tool render`).
- Pas de sélecteur de fichiers intégré : `zenity`/`kdialog` s'ils existent, sinon une zone de saisie de chemin.

**Plateforme et qualité**
- Linux uniquement testé. Pas de test sur Windows/macOS.
- Accessibilité : étiquettes et rôles Slint sur les boutons et champs, navigation clavier du canevas limitée ; **aucun test avec un lecteur de
  écran**. Saisie IME : par les widgets Slint, non testée.
- Internationalisation : textes en français marqués `@tr` (traduction anglaise à produire) ; pas de test texte de droite à gauche
  (le moteur de texte le gère, l'interface n'a pas été vérifiée).
- Interface : icônes remplacées par des glyphes/texte (aucun visuel original encore dessiné).
- Pas de fuzzing continu (`cargo-fuzz`) : un test de mutation déterministe existe (`kinaute-tool fuzz`) mais n'est pas dans la CI.
- Pas d'essai avec un vrai PowerPoint : la compatibilité est vérifiée avec python-pptx et LibreOffice seulement.

## Écarts avec le plan initial

| Plan | Réalité |
|---|---|
| `.key` lecture/écriture | Abandonné (décision du commanditaire) |
| GPU principal, CPU en repli | CPU seul |
| 11 crates | 6 crates |
| Benchmark vello/lyon/skia ; prototype comparatif d'UI | Non faits ; choix justifiés et mesurés (ADR 0002, 0003) |
| MVP : « import `.pptx` basique » | Dépassé : lecture complète du modèle, écriture sans perte |
| MVP : fondu et glissement | 9 transitions |
| Export PDF vectoriel | PDF raster (texte non sélectionnable) |
| Tests visuels de non-régression automatisés | Comparaison avec LibreOffice (outil `scripts/compare.py`) ; pas d'images de référence versionnées |
| CI multi-plateforme | CI Linux (`.github/workflows/ci.yml`), non encore exécutée sur GitHub |

## Risques mis à jour

1. **Fidélité au rendu de PowerPoint** (haut) : seules des références LibreOffice sont disponibles ; les graphiques manquent.
2. **Nom « Kinaute »** (moyen) : proximité phonétique avec « Keynote ».
3. **Texte et IME sur canevas personnalisé** (moyen) : repoussé en passant par les widgets Slint ; à rouvrir si l'on veut éditer sur la diapositive.
4. **Vieilles machines réelles** (moyen) : mesures faites sur machine récente bridée ; essai sur matériel ancien nécessaire.
5. **Ampleur** (haut) : tableaux, graphiques, animations et édition de texte riche sont chacun un chantier.
