<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<!--
  Test fragment from the Ville de Montréal terrain model (TIN 2015,
  Westmount), CityGML 2.0 module `dem`, file "TIN 2015 - Westmount.gml" in
  tin-2015-westmount.zip from
  https://donnees.montreal.ca/dataset/modele-numerique-de-terrain-mnt
  Licence: CC BY 4.0 (Ville de Montréal).

  Everything before the first triangle, the first three gml:Triangle patches
  and everything after the last one, verbatim; the other 588 656 triangles
  are dropped.

  One dem:ReliefFeature (dem:lod 2) with one dem:reliefComponent, a
  dem:TINRelief (dem:lod 2) whose dem:tin is a gml:TriangulatedSurface.
  CityJSON models it as one TINRelief City Object whose geometry is a
  CompositeSurface of triangles. The document declares no srsName anywhere.
-->
<CityModel xmlns="http://www.opengis.net/citygml/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:core="http://www.opengis.net/citygml/base/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:tex="http://www.opengis.net/citygml/textures/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.opengis.net/citygml/2.0./CityGML_2.0/CityGML.xsd">

  <!-- File Written  With RhinoCity Software  CopyRight Rhinoterraain 2012 -->

  <gml:name>Essai Rhino</gml:name>

  <gml:description>Exported by Rhinocity</gml:description>

  <app:appearanceMember>
    <app:Appearance>
      <app:theme>Rhino   texturing</app:theme>
    </app:Appearance>
  </app:appearanceMember>

  <cityObjectMember>
    <dem:ReliefFeature gml:id="6a273c64-ffe4-46c8-a4df-03adb2169573">
      <dem:lod>2</dem:lod>
      <dem:reliefComponent>
        <dem:TINRelief gml:id="UUID_eed63079-f1a9-4ca8-980c-97e45aeda5be">
          <dem:lod>2</dem:lod>
          <dem:tin>
            <gml:TriangulatedSurface gml:id="UUID_41f92240-95a1-43ae-b8e3-6c794a98cf72">
              <gml:trianglePatches>
                <gml:Triangle>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList srsDimension="3">295554.370000 5038697.350000 125.230000 295551.806739 5038696.437653 123.752867 295552.228443 5038696.169388 123.815941 295554.370000 5038697.350000 125.230000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Triangle>
                <gml:Triangle>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList srsDimension="3">295555.410000 5038696.350000 125.240000 295554.370000 5038697.350000 125.230000 295552.228443 5038696.169388 123.815941 295555.410000 5038696.350000 125.240000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Triangle>
                <gml:Triangle>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList srsDimension="3">295556.040000 5038694.840000 125.440000 295553.275597 5038695.503245 124.246160 295556.134808 5038693.684367 125.276008 295556.040000 5038694.840000 125.440000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Triangle>
              </gml:trianglePatches>
            </gml:TriangulatedSurface>
          </dem:tin>
        </dem:TINRelief>
      </dem:reliefComponent>
    </dem:ReliefFeature>
  </cityObjectMember>

  <gml:boundedBy>
    <gml:Envelope srsDimension="3">
      <gml:lowerCorner>295543.250000 5037135.000000 18.499998
</gml:lowerCorner>
      <gml:upperCorner>298499.000000 5039489.500000 203.610016
</gml:upperCorner>
    </gml:Envelope>
  </gml:boundedBy>

</CityModel>
