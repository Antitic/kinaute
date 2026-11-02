# ADR 0004 — Aller-retour sans perte

**Statut** : accepté.

**Décision** : chaque objet du modèle (forme, paragraphe, run, diapositive…) garde son XML d'origine. À l'enregistrement :
1. objet inchangé (le modèle ré-analysé depuis sa source est égal à l'objet) → réécrit **verbatim** ;
2. objet modifié → sa source est **mise à jour sur place** (seuls les champs modélisés sont réécrits, le reste est conservé) ;
3. objet nouveau → généré ;
4. partie du paquet non modélisée (graphiques, thèmes, polices incorporées, propriétés, `customXml`…) → copiée **octet pour octet**.

**Pourquoi** : PowerPoint écrit beaucoup d'extensions (`extLst`, `mc:AlternateContent`, `p14`, `a14`…). Les perdre en silence casserait la
promesse « zéro perte ».

**Vérification** : `kinaute-tool check` ouvre, enregistre, rouvre et exige un modèle identique et des parties non touchées
identiques octet pour octet (149 fichiers publics sur 149 ouvrables). Les fichiers modifiés sont relus avec python-pptx et LibreOffice.

**Limite connue** : l'égalité est testée sur le modèle, pas sur le XML : deux écritures différentes du même contenu sont considérées
identiques ; l'espacement entre formes (blancs insignifiants) n'est pas conservé.
