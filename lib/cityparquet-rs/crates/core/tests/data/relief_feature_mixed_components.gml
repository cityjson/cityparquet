<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<!--
  COMPOSED test fixture. No published CityGML 2.0 document has, at a
  committable size, one dem:ReliefFeature holding several relief components
  of different kinds, so this one is assembled to hold exactly that:

    * the root CityModel start tag: verbatim from
      montreal_tin_relief_fragment.gml (Ville de Montréal, CC BY 4.0);
    * reliefComponent 1: the dem:TINRelief of "TIN 2015 - Westmount.gml"
      (tin-2015-westmount.zip), its first three gml:Triangle patches verbatim;
    * reliefComponent 2: the first dem:TINRelief of
      "TIN 2015 - Montréal-Ouest.gml" (tin-2015-montreal-ouest.zip), its first
      three gml:Triangle patches verbatim. Both from
      https://donnees.montreal.ca/dataset/modele-numerique-de-terrain-mnt
      (CC BY 4.0, Ville de Montréal);
    * reliefComponent 3: a dem:MassPointRelief, WRITTEN FOR THIS FIXTURE —
      no published file carries one — whose three points are the corners of
      Westmount's first triangle;
    * the dem:ReliefFeature wrapper, its gml:id and dem:lod: written for this
      fixture.

  CityJSON has a TINRelief and nothing for mass points, so the two TIN
  components become two TINRelief City Objects and the mass-point component
  is reported as not mapped.
-->
<CityModel xmlns="http://www.opengis.net/citygml/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:core="http://www.opengis.net/citygml/base/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:tex="http://www.opengis.net/citygml/textures/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.opengis.net/citygml/2.0./CityGML_2.0/CityGML.xsd">
  <cityObjectMember>
    <dem:ReliefFeature gml:id="composed-relief-feature">
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
      <dem:reliefComponent>
        <dem:TINRelief gml:id="UUID_3b70702a-06ca-485b-89de-30b6891b4537">
          <dem:lod>2</dem:lod>
          <dem:tin>
            <gml:TriangulatedSurface gml:id="UUID_00b3cc92-e953-4181-b74d-bccc23c69582">
              <gml:trianglePatches>
                <gml:Triangle>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList srsDimension="3">292759.190000 5038450.300000 51.390000 292760.270000 5038447.520000 51.140000 292760.436706 5038447.699994 51.135168 292759.190000 5038450.300000 51.390000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Triangle>
                <gml:Triangle>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList srsDimension="3">292759.190000 5038450.300000 51.390000 292760.436706 5038447.699994 51.135168 292761.390470 5038449.146095 51.130595 292759.190000 5038450.300000 51.390000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Triangle>
                <gml:Triangle>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList srsDimension="3">292614.430000 5038289.400000 52.260000 292611.449279 5038287.254851 51.896873 292611.775676 5038287.042542 51.919478 292614.430000 5038289.400000 52.260000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Triangle>
              </gml:trianglePatches>
            </gml:TriangulatedSurface>
          </dem:tin>
        </dem:TINRelief>
      </dem:reliefComponent>
      <dem:reliefComponent>
        <dem:MassPointRelief gml:id="composed-mass-points">
          <dem:lod>2</dem:lod>
          <dem:reliefPoints>
            <gml:MultiPoint>
              <gml:pointMember><gml:Point><gml:pos srsDimension="3">295554.370000 5038697.350000 125.230000</gml:pos></gml:Point></gml:pointMember>
              <gml:pointMember><gml:Point><gml:pos srsDimension="3">295551.806739 5038696.437653 123.752867</gml:pos></gml:Point></gml:pointMember>
              <gml:pointMember><gml:Point><gml:pos srsDimension="3">295552.228443 5038696.169388 123.815941</gml:pos></gml:Point></gml:pointMember>
            </gml:MultiPoint>
          </dem:reliefPoints>
        </dem:MassPointRelief>
      </dem:reliefComponent>
    </dem:ReliefFeature>
  </cityObjectMember>
</CityModel>
