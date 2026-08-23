<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment from the CityGML 2.0 "Railway" reference dataset (KIT/IAI,
  GeoRES; post-processed by C. Nagel, TU Berlin) via
  github.com/cityjson/duckdb-cityjson-extension data/Railway.gml. Free for
  unrestricted use. One bldg:Building (GMLID_BUI46739_1739_10911) plus a
  Bridge, a vegetation object, and a CityObjectGroup — non-building modules the
  reader must skip. Root CityModel + envelope + 4 cityObjectMembers, verbatim.
-->
<CityModel xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:tex="http://www.opengis.net/citygml/texturedsurface/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns="http://www.opengis.net/citygml/2.0" xmlns:xal="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.opengis.net/citygml/vegetation/2.0 http://schemas.opengis.net/citygml/vegetation/2.0/vegetation.xsd http://www.opengis.net/citygml/cityobjectgroup/2.0 http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/texturedsurface/2.0 http://schemas.opengis.net/citygml/texturedsurface/2.0/texturedSurface.xsd http://www.opengis.net/citygml/waterbody/2.0 http://schemas.opengis.net/citygml/waterbody/2.0/waterBody.xsd http://www.opengis.net/citygml/appearance/2.0 http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/cityfurniture/2.0 http://schemas.opengis.net/citygml/cityfurniture/2.0/cityFurniture.xsd http://www.opengis.net/citygml/relief/2.0 http://schemas.opengis.net/citygml/relief/2.0/relief.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/bridge/2.0 http://schemas.opengis.net/citygml/bridge/2.0/bridge.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/tunnel/2.0 http://schemas.opengis.net/citygml/tunnel/2.0/tunnel.xsd">

<cityObjectMember>
    <bldg:Building gml:id="GMLID_BUI46739_1739_10911">
      <gml:description>Simple Chapel with a recess/loggia</gml:description>
      <gml:name>Chapel KIT/KHH-1</gml:name>
      <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
      <bldg:outerBuildingInstallation>
        <bldg:BuildingInstallation>
          <gml:name>Cross</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:function>1070</bldg:function>
          <bldg:lod3Geometry>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69172_144_451020_409296">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69172_144_451020_409296_0">
                      <gml:posList>10.7730103099713 6.06190876274259 8.72494585903361 10.7856888884799 6.08909798205834 8.72494585903361 10.7929393473698 6.08571702783464 8.72494585941145 10.7802607688612 6.05852780851889 8.72494585941145 10.7730103099713 6.06190876274259 8.72494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69173_390_301946_142544">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69173_390_301946_142544_0">
                      <gml:posList>10.7768798221729 6.05127735800642 8.64494585941145 10.7802607761192 6.05852781664106 8.64494585941145 10.7802607688612 6.05852780851889 8.72494585941145 10.7929393473698 6.08571702783464 8.72494585941145 10.7929393439567 6.08571702827557 8.73294585941144 10.780260765448 6.05852780895982 8.73294585941144 10.7802607653459 6.05852780469244 8.76294585941144 10.7768798113996 6.05127734605779 8.76294585941144 10.7768798115017 6.05127735032517 8.73294585941144 10.7642012342029 6.02408813044524 8.73294585941144 10.7642012376161 6.02408813000431 8.72494585941145 10.7768798205783 6.05127735875 8.72494585941145 10.7768798221729 6.05127735800642 8.64494585941145</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69174_124_50022_21497">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69174_124_50022_21497_0">
                      <gml:posList>10.7730103172293 6.06190877086476 8.64494585903361 10.7730103099713 6.06190876274259 8.72494585903361 10.7802607688612 6.05852780851889 8.72494585941145 10.7802607761192 6.05852781664106 8.64494585941145 10.7730103172293 6.06190877086476 8.64494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69175_838_556428_263377">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69175_838_556428_263377_0">
                      <gml:posList>10.7730103065581 6.06190876318351 8.73294585903361 10.773010306456 6.06190875891614 8.76294585903361 10.7802607653459 6.05852780469244 8.76294585941144 10.780260765448 6.05852780895982 8.73294585941144 10.7730103065581 6.06190876318351 8.73294585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69176_540_439031_29401">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69176_540_439031_29401_0">
                      <gml:posList>10.756950775313 6.02746908466894 8.73294585903361 10.7569507787262 6.02746908422801 8.72494585903361 10.7642012376161 6.02408813000431 8.72494585941145 10.7642012342029 6.02408813044524 8.73294585941144 10.756950775313 6.02746908466894 8.73294585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69177_1377_275588_245261">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69177_1377_275588_245261_0">
                      <gml:posList>10.773010306456 6.06190875891614 8.76294585903361 10.7696293525096 6.05465830028149 8.76294585903361 10.7768798113996 6.05127734605779 8.76294585941144 10.7802607653459 6.05852780469244 8.76294585941144 10.773010306456 6.06190875891614 8.76294585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69178_635_554748_126578">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69178_635_554748_126578_0">
                      <gml:posList>10.7696293525096 6.05465830028149 8.76294585903361 10.7696293526118 6.05465830454887 8.73294585903361 10.7768798115017 6.05127735032517 8.73294585941144 10.7768798113996 6.05127734605779 8.76294585941144 10.7696293525096 6.05465830028149 8.76294585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69179_448_878646_24779">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69179_448_878646_24779_0">
                      <gml:posList>10.769629363283 6.05465831223011 8.64494585903361 10.7730103172293 6.06190877086476 8.64494585903361 10.7802607761192 6.05852781664106 8.64494585941145 10.7768798221729 6.05127735800642 8.64494585941145 10.769629363283 6.05465831223011 8.64494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69180_1433_431186_53323">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69180_1433_431186_53323_0">
                      <gml:posList>10.7696293526118 6.05465830454887 8.73294585903361 10.756950775313 6.02746908466894 8.73294585903361 10.7642012342029 6.02408813044524 8.73294585941144 10.7768798115017 6.05127735032517 8.73294585941144 10.7696293526118 6.05465830454887 8.73294585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69181_303_425509_425">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69181_303_425509_425_0">
                      <gml:posList>10.7856888850667 6.08909798249927 8.73294585903361 10.7730103065581 6.06190876318351 8.73294585903361 10.780260765448 6.05852780895982 8.73294585941144 10.7929393439567 6.08571702827557 8.73294585941144 10.7856888850667 6.08909798249927 8.73294585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69182_1551_577110_87179">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69182_1551_577110_87179_0">
                      <gml:posList>10.7696293616883 6.0546583129737 8.72494585903361 10.769629363283 6.05465831223011 8.64494585903361 10.7768798221729 6.05127735800642 8.64494585941145 10.7768798205783 6.05127735875 8.72494585941145 10.7696293616883 6.0546583129737 8.72494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69183_472_129122_366742">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69183_472_129122_366742_0">
                      <gml:posList>10.7569507787262 6.02746908422801 8.72494585903361 10.7696293616883 6.0546583129737 8.72494585903361 10.7768798205783 6.05127735875 8.72494585941145 10.7642012376161 6.02408813000431 8.72494585941145 10.7569507787262 6.02746908422801 8.72494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69184_1787_567883_271893">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69184_1787_567883_271893_0">
                      <gml:posList>10.7730103172293 6.06190877086476 8.64494585903361 10.769629363283 6.05465831223011 8.64494585903361 10.7696293616883 6.0546583129737 8.72494585903361 10.7569507787262 6.02746908422801 8.72494585903361 10.756950775313 6.02746908466894 8.73294585903361 10.7696293526118 6.05465830454887 8.73294585903361 10.7696293525096 6.05465830028149 8.76294585903361 10.773010306456 6.06190875891614 8.76294585903361 10.7730103065581 6.06190876318351 8.73294585903361 10.7856888850667 6.08909798249927 8.73294585903361 10.7856888884799 6.08909798205834 8.72494585903361 10.7730103099713 6.06190876274259 8.72494585903361 10.7730103172293 6.06190877086476 8.64494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69185_1226_696114_428167">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69185_1226_696114_428167_0">
                      <gml:posList>10.7856888884799 6.08909798205834 8.72494585903361 10.7856888850667 6.08909798249927 8.73294585903361 10.7929393439567 6.08571702827557 8.73294585941144 10.7929393473698 6.08571702783464 8.72494585941145 10.7856888884799 6.08909798205834 8.72494585903361</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3Geometry>
        </bldg:BuildingInstallation>
      </bldg:outerBuildingInstallation>
      <bldg:outerBuildingInstallation>
        <bldg:BuildingInstallation>
          <gml:name>Tower</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:function>1040</bldg:function>
          <bldg:lod3Geometry>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69186_978_574497_355887">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69186_978_574497_355887_0">
                      <gml:posList>10.4314562023096 6.25119791974448 8.9549156695611 10.4314562332305 6.25119786636271 8.82618472548078 10.414551428758 6.21494553171688 8.85491564862439 10.3976466914325 6.17869328148492 8.82354235012055 10.3976466593627 6.17869333502236 8.95491562768779 10.4314562023096 6.25119791974448 8.9549156695611</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69187_1307_753549_366591">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69187_1307_753549_366591_0">
                      <gml:posList>10.5039600184048 6.21738883058334 8.9549157100654 10.5039600778032 6.21738876392224 8.8261847659851 10.4314562332305 6.25119786636271 8.82618472548078 10.4314562023096 6.25119791974448 8.9549156695611 10.5039600184048 6.21738883058334 8.9549157100654</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69188_38_586065_235409">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69188_38_586065_235409_0">
                      <gml:posList>10.4701505360052 6.14488417904445 8.82354228681492 10.4701504754579 6.1448842458612 8.95491556438219 10.3976466593627 6.17869333502236 8.95491562768779 10.3976466914325 6.17869328148492 8.82354235012055 10.4701505360052 6.14488417904445 8.82354228681492</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69189_1097_226401_151532">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69189_1097_226401_151532_0">
                      <gml:posList>10.5039600184048 6.21738883058334 8.9549157100654 10.4314562023096 6.25119791974448 8.9549156695611 10.4508033388837 6.19804108280287 9.03491564292412 10.5039600184048 6.21738883058334 8.9549157100654</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69190_1625_889307_78021">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69190_1625_889307_78021_0">
                      <gml:posList>10.5039600184048 6.21738883058334 8.9549157100654 10.4701504754579 6.1448842458612 8.95491556438219 10.4701505360052 6.14488417904445 8.82354228681492 10.4870552778878 6.18113643904909 8.85491565934402 10.5039600778032 6.21738876392224 8.8261847659851 10.5039600184048 6.21738883058334 8.9549157100654</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69191_1319_260846_393149">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69191_1319_260846_393149_0">
                      <gml:posList>10.5039600184048 6.21738883058334 8.9549157100654 10.4508033388837 6.19804108280287 9.03491564292412 10.4701504754579 6.1448842458612 8.95491556438219 10.5039600184048 6.21738883058334 8.9549157100654</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69192_1604_558258_373481">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69192_1604_558258_373481_0">
                      <gml:posList>10.3976466593627 6.17869333502236 8.95491562768779 10.4508033388837 6.19804108280287 9.03491564292412 10.4314562023096 6.25119791974448 8.9549156695611 10.3976466593627 6.17869333502236 8.95491562768779</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69193_253_588952_10305">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69193_253_588952_10305_0">
                      <gml:posList>10.3976466593627 6.17869333502236 8.95491562768779 10.4701504754579 6.1448842458612 8.95491556438219 10.4508033388837 6.19804108280287 9.03491564292412 10.3976466593627 6.17869333502236 8.95491562768779</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69194_1782_804696_69346">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69194_1782_804696_69346_0">
                      <gml:posList>10.4870552778878 6.18113643904909 8.85491565934402 10.414551428758 6.21494553171688 8.85491564862439 10.4314562332305 6.25119786636271 8.82618472548078 10.5039600778032 6.21738876392224 8.8261847659851 10.4870552778878 6.18113643904909 8.85491565934402</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69195_15_207525_422041">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69195_15_207525_422041_0">
                      <gml:posList>10.414551428758 6.21494553171688 8.85491564862439 10.4870552778878 6.18113643904909 8.85491565934402 10.4701505360052 6.14488417904445 8.82354228681492 10.3976466914325 6.17869328148492 8.82354235012055 10.414551428758 6.21494553171688 8.85491564862439</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3Geometry>
        </bldg:BuildingInstallation>
      </bldg:outerBuildingInstallation>
      <bldg:boundedBy>
        <bldg:OuterCeilingSurface>
          <gml:name>Outer Ceiling 1</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69196_825_631157_321273">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69196_825_631157_321273_0">
                      <gml:posList>10.8040519189003 6.03962467448152 8.79743507593159 10.7757693649638 5.9789726382397 8.74494586667155 10.7149544074207 6.0073311955207 8.74494585167422 10.7432369613572 6.06798323176253 8.79743506093425 10.8040519189003 6.03962467448152 8.79743507593159</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:OuterCeilingSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:OuterCeilingSurface>
          <gml:name>Outer Ceiling 2</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69197_222_80410_154552">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69197_222_80410_154552_0">
                      <gml:posList>10.8349356919967 6.10585492267287 8.74494582989352 10.8040519189003 6.03962467448152 8.79743507593159 10.7432369613572 6.06798323176253 8.79743506093425 10.7741207344537 6.13421347995387 8.74494581489618 10.8349356919967 6.10585492267287 8.74494582989352</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:OuterCeilingSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 1</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69198_1722_98931_285011">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_0">
                      <gml:posList>10.0799777563781 6.15998653439255 8.45494561167762 10.7208289472235 5.86115268802071 8.45494590082278 10.7208288385256 5.86115270274922 8.70494590082275 10.0799776476801 6.15998654912105 8.70494561167759 10.0799777563781 6.15998653439255 8.45494561167762</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_1">
                      <gml:posList>10.3926539088152 6.01418321055124 8.50494584096984 10.3926538664384 6.01418321880673 8.62994584096984 10.3934259047332 6.01382321135392 8.63641631766356 10.3956894142786 6.01276771927364 8.64244584319588 10.3992901406398 6.01108867260686 8.64762351536603 10.4039827002656 6.00890049560917 8.65159648437213 10.4094473029725 6.00635230892684 8.65409399894206 10.4153115451165 6.00361776727118 8.65494585758521 10.4211757882868 6.00088322513698 8.65409400754282 10.4266403940024 5.99833503705165 8.65159650098751 10.4313329584144 5.99614685782213 8.64762353886372 10.434933691013 5.99446780824676 8.64244587197456 10.4371972078222 5.99341231277934 8.636416349762 10.4379692539119 5.9930523016917 8.6299458742006 10.4379692962887 5.99305229343621 8.50494587420059 10.3926539088152 6.01418321055124 8.50494584096984</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_2">
                      <gml:posList>10.1661072172054 6.11982368508943 8.62994567483826 10.1661072595822 6.11982367683394 8.50494567483826 10.1206865954157 6.14100368597452 8.5049456415303 10.1206865530389 6.14100369423001 8.6299456415303 10.1214603845042 6.14064284947717 8.63643115047082 10.1237291522348 6.13958490438727 8.64247468382979 10.1273382434593 6.13790195616982 8.64766438479622 10.1320417045749 6.13570869496466 8.65164658382562 10.1375190024795 6.13315458789838 8.65414990063504 10.1433968683725 6.13041369314668 8.65500373832084 10.149274735407 6.12767279815735 8.65414990925577 10.1547520366581 6.12511869039437 8.6516466004796 10.1594555030974 6.12292542808093 8.6476644083485 10.1630646012598 6.12124247841914 8.64247471267533 10.1653333770698 6.12018453164725 8.63643118264384 10.1661072172054 6.11982368508943 8.62994567483826</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_3">
                      <gml:posList>10.256954835751 6.07746073363319 8.6299457414588 10.2569548781278 6.0774607253777 8.50494574145879 10.2116618912598 6.09858119760047 8.50494570824446 10.211661848883 6.09858120585596 8.62994570824446 10.2124310881119 6.09822249963914 8.63641539666842 10.2146921594239 6.09716814078407 8.64244475412761 10.2182908780803 6.09549002708711 8.64762263264076 10.2229818438171 6.09330259081797 8.6515959474368 10.2284451749379 6.09075499546752 8.65409375416245 10.2343083213698 6.08802096413851 8.65494572485162 10.2401714692293 6.0852869331916 8.6540937627616 10.2456348045359 6.08273933896137 8.6515959640487 10.2503257769311 6.0805519044742 8.64762265613265 10.2539245042644 6.07887379309945 8.64244478289755 10.2561855856803 6.07781943694846 8.63641542875454 10.256954835751 6.07746073363319 8.6299457414588</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_4">
                      <gml:posList>10.3473384794739 6.03531413701339 8.62994580773908 10.3473385218507 6.0353141287579 8.50494580773908 10.3024196510437 6.05626014798404 8.5049457747991 10.3024196086669 6.05626015623953 8.6299457747991 10.3031848910888 6.05590329799554 8.63635963386839 10.3054285941851 6.05485704075347 8.64233640016392 10.3089978132977 6.05319268521536 8.64746876693971 10.3136493120425 6.05102365446064 8.65140697188164 10.319066098467 6.04849776435077 8.6538826328311 10.324879027528 6.04578715012249 8.65472703758561 10.3306919577163 6.04307653565596 8.65388264135659 10.336108747446 6.04055064484757 8.65140698835163 10.3407602514486 6.03838161298169 8.6474687902318 10.3443294774133 6.03671725599548 8.64233642869078 10.346573188489 6.03567099706706 8.63635966568597 10.3473384794739 6.03531413701339 8.62994580773908</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_5">
                      <gml:posList>10.5286000284071 5.95079046649263 8.62994594066211 10.528600070784 5.95079045823714 8.50494594066211 10.4832846835689 5.97192137590652 8.50494590743135 10.4832846411921 5.97192138416201 8.62994590743135 10.4840566794825 5.97156137669976 8.63641638412507 10.486320189015 5.97050588459178 8.64244590965739 10.4899209153556 5.96882683788095 8.64762358182754 10.4946134749547 5.96663866082586 8.65159655083365 10.5000780776304 5.96409047407668 8.65409406540358 10.505942319741 5.96135593234928 8.65494592404673 10.5118065628778 5.95862139014335 8.65409407400433 10.5172711685622 5.95607320199117 8.65159656744903 10.5219637329475 5.95388502270424 8.64762360532524 10.5255644655256 5.95220597308482 8.64244593843607 10.5278279823218 5.95115047758972 8.63641641622352 10.5286000284071 5.95079046649263 8.62994594066211</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69198_1722_98931_285011_6">
                      <gml:posList>10.6192308028318 5.90852863114228 8.62994600712362 10.6192308452086 5.90852862288679 8.50494600712362 10.5739154579972 5.929659540564 8.50494597389286 10.5739154156204 5.92965954881949 8.62994597389287 10.5746874539108 5.9292995413571 8.63641645058658 10.5769509634431 5.92824404924873 8.6424459761189 10.5805516897834 5.92656500253728 8.64762364828906 10.5852442493821 5.92437682548138 8.65159661729516 10.5907088520573 5.92182863873126 8.65409413186509 10.5965730941675 5.91909409700285 8.65494599050824 10.6024373373038 5.9163595547959 8.65409414046584 10.6079019429878 5.91381136664278 8.65159663391054 10.6125945073727 5.91162318735504 8.64762367178675 10.6161952399505 5.909944137735 8.64244600489758 10.6184587567466 5.9088886422395 8.63641648268502 10.6192308028318 5.90852863114228 8.62994600712362</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 1.1</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69199_1168_381032_18830">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69199_1168_381032_18830_0">
                          <gml:posList>10.1661072595822 6.11982367683394 8.50494567483826 10.1661072172054 6.11982368508943 8.62994567483826 10.1653333770698 6.12018453164725 8.63643118264384 10.1630646012598 6.12124247841914 8.64247471267533 10.1594555030974 6.12292542808093 8.6476644083485 10.1547520366581 6.12511869039437 8.6516466004796 10.149274735407 6.12767279815735 8.65414990925577 10.1433968683725 6.13041369314668 8.65500373832084 10.1375190024795 6.13315458789838 8.65414990063504 10.1320417045749 6.13570869496466 8.65164658382562 10.1273382434593 6.13790195616982 8.64766438479622 10.1237291522348 6.13958490438727 8.64247468382979 10.1214603845042 6.14064284947717 8.63643115047082 10.1206865530389 6.14100369423001 8.6299456415303 10.1206865954157 6.14100368597452 8.5049456415303 10.1661072595822 6.11982367683394 8.50494567483826</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 1.2</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69200_944_344165_81802">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69200_944_344165_81802_0">
                          <gml:posList>10.2569548781278 6.0774607253777 8.50494574145879 10.256954835751 6.07746073363319 8.6299457414588 10.2561855856803 6.07781943694846 8.63641542875454 10.2539245042644 6.07887379309945 8.64244478289755 10.2503257769311 6.0805519044742 8.64762265613265 10.2456348045359 6.08273933896137 8.6515959640487 10.2401714692293 6.0852869331916 8.6540937627616 10.2343083213698 6.08802096413851 8.65494572485162 10.2284451749379 6.09075499546752 8.65409375416245 10.2229818438171 6.09330259081797 8.6515959474368 10.2182908780803 6.09549002708711 8.64762263264076 10.2146921594239 6.09716814078407 8.64244475412761 10.2124310881119 6.09822249963914 8.63641539666842 10.211661848883 6.09858120585596 8.62994570824446 10.2116618912598 6.09858119760047 8.50494570824446 10.2569548781278 6.0774607253777 8.50494574145879</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 1.3</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69201_1122_153071_302905">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69201_1122_153071_302905_0">
                          <gml:posList>10.3473385218507 6.0353141287579 8.50494580773908 10.3473384794739 6.03531413701339 8.62994580773908 10.346573188489 6.03567099706706 8.63635966568597 10.3443294774133 6.03671725599548 8.64233642869078 10.3407602514486 6.03838161298169 8.6474687902318 10.336108747446 6.04055064484757 8.65140698835163 10.3306919577163 6.04307653565596 8.65388264135659 10.324879027528 6.04578715012249 8.65472703758561 10.319066098467 6.04849776435077 8.6538826328311 10.3136493120425 6.05102365446064 8.65140697188164 10.3089978132977 6.05319268521536 8.64746876693971 10.3054285941851 6.05485704075347 8.64233640016392 10.3031848910888 6.05590329799554 8.63635963386839 10.3024196086669 6.05626015623953 8.6299457747991 10.3024196510437 6.05626014798404 8.5049457747991 10.3473385218507 6.0353141287579 8.50494580773908</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 1.4</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69202_502_27919_246134">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69202_502_27919_246134_0">
                          <gml:posList>10.3926538664384 6.01418321880673 8.62994584096984 10.3926539088152 6.01418321055124 8.50494584096984 10.4379692962887 5.99305229343621 8.50494587420059 10.4379692539119 5.9930523016917 8.6299458742006 10.4371972078222 5.99341231277934 8.636416349762 10.434933691013 5.99446780824676 8.64244587197456 10.4313329584144 5.99614685782213 8.64762353886372 10.4266403940024 5.99833503705165 8.65159650098751 10.4211757882868 6.00088322513698 8.65409400754282 10.4153115451165 6.00361776727118 8.65494585758521 10.4094473029725 6.00635230892684 8.65409399894206 10.4039827002656 6.00890049560917 8.65159648437213 10.3992901406398 6.01108867260686 8.64762351536603 10.3956894142786 6.01276771927364 8.64244584319588 10.3934259047332 6.01382321135392 8.63641631766356 10.3926538664384 6.01418321880673 8.62994584096984</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 1.5</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69203_1511_897079_25059">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69203_1511_897079_25059_0">
                          <gml:posList>10.528600070784 5.95079045823714 8.50494594066211 10.5286000284071 5.95079046649263 8.62994594066211 10.5278279823218 5.95115047758972 8.63641641622352 10.5255644655256 5.95220597308482 8.64244593843607 10.5219637329475 5.95388502270424 8.64762360532524 10.5172711685622 5.95607320199117 8.65159656744903 10.5118065628778 5.95862139014335 8.65409407400433 10.505942319741 5.96135593234928 8.65494592404673 10.5000780776304 5.96409047407668 8.65409406540358 10.4946134749547 5.96663866082586 8.65159655083365 10.4899209153556 5.96882683788095 8.64762358182754 10.486320189015 5.97050588459178 8.64244590965739 10.4840566794825 5.97156137669976 8.63641638412507 10.4832846411921 5.97192138416201 8.62994590743135 10.4832846835689 5.97192137590652 8.50494590743135 10.528600070784 5.95079045823714 8.50494594066211</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 1.6</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69204_1957_398666_57160">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69204_1957_398666_57160_0">
                          <gml:posList>10.6192308452086 5.90852862288679 8.50494600712362 10.6192308028318 5.90852863114228 8.62994600712362 10.6184587567466 5.9088886422395 8.63641648268502 10.6161952399505 5.909944137735 8.64244600489758 10.6125945073727 5.91162318735504 8.64762367178675 10.6079019429878 5.91381136664278 8.65159663391054 10.6024373373038 5.9163595547959 8.65409414046584 10.5965730941675 5.91909409700285 8.65494599050824 10.5907088520573 5.92182863873126 8.65409413186509 10.5852442493821 5.92437682548138 8.65159661729516 10.5805516897834 5.92656500253728 8.64762364828906 10.5769509634431 5.92824404924873 8.6424459761189 10.5746874539108 5.9292995413571 8.63641645058658 10.5739154156204 5.92965954881949 8.62994597389287 10.5739154579972 5.929659540564 8.50494597389286 10.6192308452086 5.90852862288679 8.50494600712362</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 2</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69205_1118_733152_315294">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69205_1118_733152_315294_0">
                      <gml:posList>10.7208289472235 5.86115268802071 8.45494590082278 10.7757694559817 5.9789726375089 8.45494586667155 10.7757693832651 5.97897264696563 8.62494586667155 10.8349357323167 6.10585492113129 8.62494582989352 10.8349358050333 6.10585491167457 8.45494582989353 10.8898763123263 6.22367486184599 8.4549457957423 10.8898762036283 6.22367487657448 8.70494579574227 10.8016364794322 6.03444488483946 8.85491564617534 10.7208288385256 5.86115270274922 8.70494590082275 10.7208289472235 5.86115268802071 8.45494590082278</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69205_1118_733152_315294_1">
                      <gml:posList>10.7630906250027 5.95178305434068 8.62994587455271 10.7630906793517 5.95178304697642 8.50494587455271 10.7419597150099 5.90646768152511 8.5049458876878 10.7419596606609 5.90646768888937 8.6299458876878 10.7423196689973 5.90723973339264 8.63641636359158 10.7433751635101 5.90950324797305 8.64244588680791 10.7450542139927 5.91310397785256 8.64762355529387 10.7472423959293 5.91779653924038 8.65159651949864 10.7497905883372 5.92326114183275 8.65409402847729 10.7525251361149 5.92912538199368 8.65494588112025 10.7552596843503 5.93498962144733 8.65409402507767 10.7578078781002 5.94045422196603 8.65159651293109 10.7599960621718 5.94514678005513 8.64762354600596 10.7616751154366 5.94874750563566 8.64244587543259 10.7627306131894 5.95101101520979 8.63641635090406 10.7630906250027 5.95178305434068 8.62994587455271</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69205_1118_733152_315294_2">
                      <gml:posList>10.8687454467117 6.17835988159724 8.62994580887728 10.8687455010607 6.17835987423299 8.50494580887728 10.8476145367189 6.13304450878167 8.50494582201237 10.8476144823699 6.13304451614593 8.62994582201236 10.8479744919347 6.13381656007638 8.63641629791614 10.8490299875922 6.136080074123 8.64244582113248 10.8507090390578 6.13968080354414 8.64762348961844 10.8528972217487 6.14437336458023 8.6515964538232 10.8554454146307 6.1498379669515 8.65409396280185 10.8581799625701 6.15570220703702 8.65494581544482 10.8609145106438 6.16156644656608 8.65409395940224 10.8634627039196 6.16703104730589 8.65159644725566 10.8656508872369 6.17172360574671 8.64762348033053 10.8673299395187 6.17532433178561 8.64244580975716 10.8683854361268 6.17758784189353 8.63641628522862 10.8687454467117 6.17835988159724 8.62994580887728</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69205_1118_733152_315294_3">
                      <gml:posList>10.8349357237618 6.10585492224385 8.64494586667155 10.7757693736132 5.97897264858974 8.64494586667155 10.7757693649638 5.9789726382397 8.74494586667155 10.8040519189003 6.03962467448152 8.79743507593159 10.8349356919967 6.10585492267287 8.74494582989352 10.8349357237618 6.10585492224385 8.64494586667155</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69206_1639_367589_148340">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69206_1639_367589_148340_0">
                      <gml:posList>10.7149544160701 6.00733120587075 8.64494585167422 10.7741207662188 6.13421347952486 8.64494585167422 10.7741207344537 6.13421347995387 8.74494581489618 10.7432369613572 6.06798323176253 8.79743506093425 10.7149544074207 6.0073311955207 8.74494585167422 10.7149544160701 6.00733120587075 8.64494585167422</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 2.1</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69207_292_816317_401457">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69207_292_816317_401457_0">
                          <gml:posList>10.7630906793517 5.95178304697642 8.50494587455271 10.7630906250027 5.95178305434068 8.62994587455271 10.7627306131894 5.95101101520979 8.63641635090406 10.7616751154366 5.94874750563566 8.64244587543259 10.7599960621718 5.94514678005513 8.64762354600596 10.7578078781002 5.94045422196603 8.65159651293109 10.7552596843503 5.93498962144733 8.65409402507767 10.7525251361149 5.92912538199368 8.65494588112025 10.7497905883372 5.92326114183275 8.65409402847729 10.7472423959293 5.91779653924038 8.65159651949864 10.7450542139927 5.91310397785256 8.64762355529387 10.7433751635101 5.90950324797305 8.64244588680791 10.7423196689973 5.90723973339264 8.63641636359158 10.7419596606609 5.90646768888937 8.6299458876878 10.7419597150099 5.90646768152511 8.5049458876878 10.7630906793517 5.95178304697642 8.50494587455271</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 2.2</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69208_703_561410_245316">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69208_703_561410_245316_0">
                          <gml:posList>10.8687455010607 6.17835987423299 8.50494580887728 10.8687454467117 6.17835988159724 8.62994580887728 10.8683854361268 6.17758784189353 8.63641628522862 10.8673299395187 6.17532433178561 8.64244580975716 10.8656508872369 6.17172360574671 8.64762348033053 10.8634627039196 6.16703104730589 8.65159644725566 10.8609145106438 6.16156644656608 8.65409395940224 10.8581799625701 6.15570220703702 8.65494581544482 10.8554454146307 6.1498379669515 8.65409396280185 10.8528972217487 6.14437336458023 8.6515964538232 10.8507090390578 6.13968080354414 8.64762348961844 10.8490299875922 6.136080074123 8.64244582113248 10.8479744919347 6.13381656007638 8.63641629791614 10.8476144823699 6.13304451614593 8.62994582201236 10.8476145367189 6.13304450878167 8.50494582201237 10.8687455010607 6.17835987423299 8.50494580887728</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 3</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69209_1219_606441_272939">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_0">
                      <gml:posList>10.2490249853852 6.52250866419215 8.70494582104368 10.8898762036283 6.22367487657448 8.70494579574227 10.8898763123263 6.22367486184599 8.4549457957423 10.2490250940832 6.52250864946366 8.4549458210437 10.2490249853852 6.52250866419215 8.70494582104368</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_1">
                      <gml:posList>10.7429627619295 6.29218170712501 8.62994591548768 10.7429628162785 6.29218169976076 8.50494591548769 10.7882782056304 6.27105078667373 8.50494592415227 10.7882781512814 6.27105079403798 8.62994592415227 10.7875061078626 6.27141080333123 8.63641640013221 10.7852425934785 6.27246629698929 8.64244592357185 10.7816418628935 6.27414534486364 8.64762359241304 10.7769492999467 6.27633352261641 8.65159655708073 10.7714846950479 6.27888170954988 8.65409406659848 10.7656204519904 6.28161625093561 8.65494591981998 10.7597562092474 6.28435079229721 8.65409406435592 10.7542916052708 6.28689897915992 8.65159655274844 10.7495990437907 6.28908715680014 8.64762358628625 10.7459983151174 6.29076620452782 8.6424459160681 10.7437348029594 6.29182169801506 8.63641639176287 10.7429627619295 6.29218170712501 8.62994591548768</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_2">
                      <gml:posList>10.6525907924428 6.33432284857923 8.629945898208 10.6525908467918 6.33432284121498 8.504945898208 10.6979062361436 6.31319192812795 8.50494590687259 10.6979061817946 6.3131919354922 8.62994590687258 10.6971341383759 6.31355194478545 8.63641638285253 10.6948706239917 6.31460743844351 8.64244590629217 10.6912698934068 6.31628648631786 8.64762357513335 10.6865773304599 6.31847466407063 8.65159653980105 10.6811127255612 6.3210228510041 8.6540940493188 10.6752484825037 6.32375739238983 8.65494590254029 10.6693842397607 6.32649193375143 8.65409404707624 10.663919635784 6.32904012061414 8.65159653546876 10.659227074304 6.33122829825436 8.64762356900656 10.6556263456306 6.33290734598203 8.64244589878841 10.6533628334726 6.33396283946928 8.63641637448318 10.6525907924428 6.33432284857923 8.629945898208</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_3">
                      <gml:posList>10.5784946518401 6.36887444464533 8.65409402969758 10.5730300478634 6.37142263150805 8.6515965180901 10.5683374863834 6.37361080914827 8.6476235516279 10.5647367577101 6.37528985687594 8.64244588140976 10.562473245552 6.37634535036319 8.63641635710452 10.5617012045222 6.37670535947314 8.62994588082934 10.5617012588712 6.37670535210889 8.50494588082934 10.607016648223 6.35557443902186 8.50494588949393 10.607016593874 6.35557444638611 8.62994588949392 10.6062445504553 6.35593445567936 8.63641636547387 10.6039810360711 6.35698994933741 8.64244588891351 10.6003803054862 6.35866899721177 8.64762355775469 10.5956877425393 6.36085717496454 8.65159652242239 10.5902231376406 6.36340536189801 8.65409403194014 10.5843588945831 6.36613990328374 8.65494588516163 10.5784946518401 6.36887444464533 8.65409402969758</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_4">
                      <gml:posList>10.4710704258185 6.4189671856472 8.62994586350016 10.4710704801675 6.41896717828295 8.50494586350017 10.5163858695194 6.39783626519592 8.50494587216476 10.5163858151704 6.39783627256017 8.62994587216475 10.5156137717516 6.39819628185342 8.6364163481447 10.5133502573675 6.39925177551148 8.64244587158433 10.5097495267825 6.40093082338583 8.64762354042552 10.5050569638357 6.4031190011386 8.65159650509322 10.4995923589369 6.40566718807207 8.65409401461097 10.4937281158794 6.4084017294578 8.65494586783246 10.4878638731364 6.4111362708194 8.65409401236841 10.4823992691598 6.41368445768211 8.65159650076092 10.4777067076797 6.41587263532233 8.64762353429873 10.4741059790064 6.41755168305001 8.64244586408058 10.4718424668484 6.41860717653725 8.63641633977535 10.4710704258185 6.4189671856472 8.62994586350016</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_5">
                      <gml:posList>10.3812116881447 6.46086900271131 8.63641632244618 10.3804396471149 6.46122901182126 8.62994584617099 10.3804397014639 6.46122900445701 8.504945846171 10.4257550908157 6.44009809136998 8.50494585483558 10.4257550364667 6.44009809873423 8.62994585483558 10.424982993048 6.44045810802748 8.63641633081552 10.4227194786638 6.44151360168554 8.64244585425516 10.4191187480789 6.44319264955989 8.64762352309634 10.414426185132 6.44538082731266 8.65159648776404 10.4089615802333 6.44792901424613 8.65409399728179 10.4030973371758 6.45066355563186 8.65494585050329 10.3972330944328 6.45339809699346 8.65409399503923 10.3917684904561 6.45594628385618 8.65159648343175 10.3870759289761 6.45813446149639 8.64762351696956 10.3834752003027 6.45981350922407 8.64244584675141 10.3812116881447 6.46086900271131 8.63641632244618</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69209_1219_606441_272939_6">
                      <gml:posList>10.3183308015296 6.49019084042019 8.65409397995262 10.3124665584721 6.49292538180592 8.65494583317411 10.3066023157291 6.49565992316752 8.65409397771006 10.3011377117524 6.49820811003024 8.65159646610258 10.2964451502724 6.50039628767046 8.64762349964038 10.2928444215991 6.50207533539813 8.64244582942224 10.290580909441 6.50313082888538 8.636416305117 10.2898088684112 6.50349083799532 8.62994582884182 10.2898089227602 6.50349083063107 8.50494582884182 10.335124312112 6.48235991754404 8.50494583750641 10.335124257763 6.48235992490829 8.62994583750641 10.3343522143443 6.48271993420154 8.63641631348635 10.3320886999601 6.4837754278596 8.64244583692599 10.3284879693752 6.48545447573395 8.64762350576717 10.3237954064283 6.48764265348672 8.65159647043487 10.3183308015296 6.49019084042019 8.65409397995262</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 3.1</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69210_370_329498_388789">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69210_370_329498_388789_0">
                          <gml:posList>10.7429628162785 6.29218169976076 8.50494591548769 10.7429627619295 6.29218170712501 8.62994591548768 10.7437348029594 6.29182169801506 8.63641639176287 10.7459983151174 6.29076620452782 8.6424459160681 10.7495990437907 6.28908715680014 8.64762358628625 10.7542916052708 6.28689897915992 8.65159655274844 10.7597562092474 6.28435079229721 8.65409406435592 10.7656204519904 6.28161625093561 8.65494591981998 10.7714846950479 6.27888170954988 8.65409406659848 10.7769492999467 6.27633352261641 8.65159655708073 10.7816418628935 6.27414534486364 8.64762359241304 10.7852425934785 6.27246629698929 8.64244592357185 10.7875061078626 6.27141080333123 8.63641640013221 10.7882781512814 6.27105079403798 8.62994592415227 10.7882782056304 6.27105078667373 8.50494592415227 10.7429628162785 6.29218169976076 8.50494591548769</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 3.2</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69211_438_240361_350478">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69211_438_240361_350478_0">
                          <gml:posList>10.6525908467918 6.33432284121498 8.504945898208 10.6525907924428 6.33432284857923 8.629945898208 10.6533628334726 6.33396283946928 8.63641637448318 10.6556263456306 6.33290734598203 8.64244589878841 10.659227074304 6.33122829825436 8.64762356900656 10.663919635784 6.32904012061414 8.65159653546876 10.6693842397607 6.32649193375143 8.65409404707624 10.6752484825037 6.32375739238983 8.65494590254029 10.6811127255612 6.3210228510041 8.6540940493188 10.6865773304599 6.31847466407063 8.65159653980105 10.6912698934068 6.31628648631786 8.64762357513335 10.6948706239917 6.31460743844351 8.64244590629217 10.6971341383759 6.31355194478545 8.63641638285253 10.6979061817946 6.3131919354922 8.62994590687258 10.6979062361436 6.31319192812795 8.50494590687259 10.6525908467918 6.33432284121498 8.504945898208</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 3.3</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69212_196_394013_102959">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69212_196_394013_102959_0">
                          <gml:posList>10.5730300478634 6.37142263150805 8.6515965180901 10.5784946518401 6.36887444464533 8.65409402969758 10.5843588945831 6.36613990328374 8.65494588516163 10.5902231376406 6.36340536189801 8.65409403194014 10.5956877425393 6.36085717496454 8.65159652242239 10.6003803054862 6.35866899721177 8.64762355775469 10.6039810360711 6.35698994933741 8.64244588891351 10.6062445504553 6.35593445567936 8.63641636547387 10.607016593874 6.35557444638611 8.62994588949392 10.607016648223 6.35557443902186 8.50494588949393 10.5617012588712 6.37670535210889 8.50494588082934 10.5617012045222 6.37670535947314 8.62994588082934 10.562473245552 6.37634535036319 8.63641635710452 10.5647367577101 6.37528985687594 8.64244588140976 10.5683374863834 6.37361080914827 8.6476235516279 10.5730300478634 6.37142263150805 8.6515965180901</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 3.4</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69213_959_795649_414661">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69213_959_795649_414661_0">
                          <gml:posList>10.4710704801675 6.41896717828295 8.50494586350017 10.4710704258185 6.4189671856472 8.62994586350016 10.4718424668484 6.41860717653725 8.63641633977535 10.4741059790064 6.41755168305001 8.64244586408058 10.4777067076797 6.41587263532233 8.64762353429873 10.4823992691598 6.41368445768211 8.65159650076092 10.4878638731364 6.4111362708194 8.65409401236841 10.4937281158794 6.4084017294578 8.65494586783246 10.4995923589369 6.40566718807207 8.65409401461097 10.5050569638357 6.4031190011386 8.65159650509322 10.5097495267825 6.40093082338583 8.64762354042552 10.5133502573675 6.39925177551148 8.64244587158433 10.5156137717516 6.39819628185342 8.6364163481447 10.5163858151704 6.39783627256017 8.62994587216475 10.5163858695194 6.39783626519592 8.50494587216476 10.4710704801675 6.41896717828295 8.50494586350017</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 3.5</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69214_1264_482708_226925">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69214_1264_482708_226925_0">
                          <gml:posList>10.3804396471149 6.46122901182126 8.62994584617099 10.3812116881447 6.46086900271131 8.63641632244618 10.3834752003027 6.45981350922407 8.64244584675141 10.3870759289761 6.45813446149639 8.64762351696956 10.3917684904561 6.45594628385618 8.65159648343175 10.3972330944328 6.45339809699346 8.65409399503923 10.4030973371758 6.45066355563186 8.65494585050329 10.4089615802333 6.44792901424613 8.65409399728179 10.414426185132 6.44538082731266 8.65159648776404 10.4191187480789 6.44319264955989 8.64762352309634 10.4227194786638 6.44151360168554 8.64244585425516 10.424982993048 6.44045810802748 8.63641633081552 10.4257550364667 6.44009809873423 8.62994585483558 10.4257550908157 6.44009809136998 8.50494585483558 10.3804397014639 6.46122900445701 8.504945846171 10.3804396471149 6.46122901182126 8.62994584617099</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 3.6</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69215_1968_573931_76641">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69215_1968_573931_76641_0">
                          <gml:posList>10.3124665584721 6.49292538180592 8.65494583317411 10.3183308015296 6.49019084042019 8.65409397995262 10.3237954064283 6.48764265348672 8.65159647043487 10.3284879693752 6.48545447573395 8.64762350576717 10.3320886999601 6.4837754278596 8.64244583692599 10.3343522143443 6.48271993420154 8.63641631348635 10.335124257763 6.48235992490829 8.62994583750641 10.335124312112 6.48235991754404 8.50494583750641 10.2898089227602 6.50349083063107 8.50494582884182 10.2898088684112 6.50349083799532 8.62994582884182 10.290580909441 6.50313082888538 8.636416305117 10.2928444215991 6.50207533539813 8.64244582942224 10.2964451502724 6.50039628767046 8.64762349964038 10.3011377117524 6.49820811003024 8.65159646610258 10.3066023157291 6.49565992316752 8.65409397771006 10.3124665584721 6.49292538180592 8.65494583317411</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:GroundSurface>
          <gml:name>Ground 1</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69216_186_414382_86668">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69216_186_414382_86668_0">
                      <gml:posList>10.2490250940832 6.52250864946366 8.4549458210437 10.8898763123263 6.22367486184599 8.4549457957423 10.8349358050333 6.10585491167457 8.45494582989353 10.774120793816 6.13421337394904 8.45494596610148 10.7656685828994 6.11608760265913 8.45494597135541 10.7234066542158 6.02545687175651 8.45494599762558 10.7149544447643 6.00733109978337 8.45494600287951 10.7757694559817 5.9789726375089 8.45494586667155 10.7208289472235 5.86115268802071 8.45494590082278 10.0799777563781 6.15998653439255 8.45494561167762 10.0316089106388 6.29287910064958 8.45494560345362 10.1161323909759 6.47413975391326 8.4549455082546 10.2490250940832 6.52250864946366 8.4549458210437</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:GroundSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 4</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69217_1826_705440_900">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69217_1826_705440_900_0">
                      <gml:posList>10.1161323909759 6.47413975391326 8.4549455082546 10.1161322822779 6.47413976864175 8.70494550825457 10.2490249853852 6.52250866419215 8.70494582104368 10.2490250940832 6.52250864946366 8.4549458210437 10.1161323909759 6.47413975391326 8.4549455082546</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69217_1826_705440_900_1">
                      <gml:posList>10.206071041494 6.50687469397324 8.50494571994294 10.206070987145 6.50687470133748 8.62994571994294 10.2052704920371 6.50658334625245 8.6364161941864 10.2029235865091 6.50572914483243 8.64244571253496 10.1991902082944 6.50437030949254 8.64762337327741 10.1943247809486 6.50259944261563 8.65159632739063 10.1886588753048 6.50053722585285 8.65409382461744 10.1825786134693 6.49832419586862 8.65494566464912 10.1764983552381 6.49611116699916 8.65409379599526 10.1708324601615 6.49404895350471 8.65159627209684 10.1659670496257 6.49227809182696 8.64762329508017 10.1622336933182 6.49091926326276 8.6424456167633 10.1598868133017 6.49006506973318 8.63641608736699 10.1590863455709 6.48977372311565 8.62994560935534 10.1590863999198 6.48977371575141 8.50494560935535 10.206071041494 6.50687469397324 8.50494571994294</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 4.1</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69218_248_244662_213227">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69218_248_244662_213227_0">
                          <gml:posList>10.206070987145 6.50687470133748 8.62994571994294 10.206071041494 6.50687469397324 8.50494571994294 10.1590863999198 6.48977371575141 8.50494560935535 10.1590863455709 6.48977372311565 8.62994560935534 10.1598868133017 6.49006506973318 8.63641608736699 10.1622336933182 6.49091926326276 8.6424456167633 10.1659670496257 6.49227809182696 8.64762329508017 10.1708324601615 6.49404895350471 8.65159627209684 10.1764983552381 6.49611116699916 8.65409379599526 10.1825786134693 6.49832419586862 8.65494566464912 10.1886588753048 6.50053722585285 8.65409382461744 10.1943247809486 6.50259944261563 8.65159632739063 10.1991902082944 6.50437030949254 8.64762337327741 10.2029235865091 6.50572914483243 8.64244571253496 10.2052704920371 6.50658334625245 8.6364161941864 10.206070987145 6.50687470133748 8.62994571994294</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:OuterCeilingSurface>
          <gml:name>Outer Ceiling 3</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69219_135_612335_111387">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69219_135_612335_111387_0">
                      <gml:posList>10.7741207210994 6.13421338340576 8.62494596610148 10.8349357323167 6.10585492113129 8.62494582989352 10.7757693832651 5.97897264696563 8.62494586667155 10.7149543720477 6.0073311092401 8.62494600287951 10.7741207210994 6.13421338340576 8.62494596610148</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:OuterCeilingSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 5</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69220_1969_559499_369655">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69220_1969_559499_369655_0">
                      <gml:posList>10.0316089106388 6.29287910064958 8.45494560345362 10.0316088019408 6.29287911537808 8.7049456034536 10.1161322822779 6.47413976864175 8.70494550825457 10.1161323909759 6.47413975391326 8.4549455082546 10.0316089106388 6.29287910064958 8.45494560345362</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 6</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69221_692_818903_166134">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69221_692_818903_166134_0">
                      <gml:posList>10.7757693649638 5.9789726382397 8.74494586667155 10.7757693736132 5.97897264858974 8.64494586667155 10.7149544160701 6.00733120587075 8.64494585167422 10.7149544074207 6.0073311955207 8.74494585167422 10.7757693649638 5.9789726382397 8.74494586667155</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:OuterFloorSurface>
          <gml:name>Outer Floor 1</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69222_1822_191125_197171">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69222_1822_191125_197171_0">
                      <gml:posList>10.7757693736132 5.97897264858974 8.64494586667155 10.8349357237618 6.10585492224385 8.64494586667155 10.7741207662188 6.13421347952486 8.64494585167422 10.7149544160701 6.00733120587075 8.64494585167422 10.7757693736132 5.97897264858974 8.64494586667155</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:OuterFloorSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 7</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69223_997_447232_328862">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69223_997_447232_328862_0">
                      <gml:posList>10.0799776476801 6.15998654912105 8.70494561167759 10.0316088019408 6.29287911537808 8.7049456034536 10.0316089106388 6.29287910064958 8.45494560345362 10.0799777563781 6.15998653439255 8.45494561167762 10.0799776476801 6.15998654912105 8.70494561167759</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                  <gml:interior>
                    <gml:LinearRing gml:id="PolyID69223_997_447232_328862_1">
                      <gml:posList>10.0472428237439 6.24992514164914 8.5049456061118 10.0472427703696 6.24992514936814 8.62994560611179 10.0475341189708 6.24912466752167 8.63641608231888 10.0483883143539 6.24677777433677 8.64244560636034 10.0497471445147 6.24304440670707 8.64762327613312 10.0515180074224 6.23817898746925 8.6515962419954 10.0535802216952 6.23251308690702 8.65409375288673 10.0557932508371 6.22643282678398 8.6549456075656 10.0580062805677 6.22035256678332 8.65409375354286 10.0600684965668 6.2146866665799 8.65159624326295 10.0618393622207 6.20982124791288 8.64762327792571 10.0631981959604 6.20608788102705 8.64244560855579 10.0640523955112 6.2037409887084 8.63641608476759 10.0643437485849 6.20294050779155 8.62994560864689 10.0643438019592 6.20294050007255 8.50494560901943 10.0472428237439 6.24992514164914 8.5049456061118</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
          <bldg:opening>
            <bldg:Window>
              <gml:name>Window 7.1</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69224_1110_379632_57164">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69224_1110_379632_57164_0">
                          <gml:posList>10.0472427703696 6.24992514936814 8.62994560611179 10.0472428237439 6.24992514164914 8.5049456061118 10.0643438019592 6.20294050007255 8.50494560901943 10.0643437485849 6.20294050779155 8.62994560864689 10.0640523955112 6.2037409887084 8.63641608476759 10.0631981959604 6.20608788102705 8.64244560855579 10.0618393622207 6.20982124791288 8.64762327792571 10.0600684965668 6.2146866665799 8.65159624326295 10.0580062805677 6.22035256678332 8.65409375354286 10.0557932508371 6.22643282678398 8.6549456075656 10.0535802216952 6.23251308690702 8.65409375288673 10.0515180074224 6.23817898746925 8.6515962419954 10.0497471445147 6.24304440670707 8.64762327613312 10.0483883143539 6.24677777433677 8.64244560636034 10.0475341189708 6.24912466752167 8.63641608231888 10.0472427703696 6.24992514936814 8.62994560611179</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Window>
          </bldg:opening>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 8</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69225_1555_894393_344958">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69225_1555_894393_344958_0">
                      <gml:posList>10.8349357237618 6.10585492224385 8.64494586667155 10.8349356919967 6.10585492267287 8.74494582989352 10.7741207344537 6.13421347995387 8.74494581489618 10.7741207662188 6.13421347952486 8.64494585167422 10.8349357237618 6.10585492224385 8.64494586667155</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 9</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69226_318_564786_45992">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69226_318_564786_45992_0">
                      <gml:posList>10.7149543720477 6.0073311092401 8.62494600287951 10.7757693832651 5.97897264696563 8.62494586667155 10.7757694559817 5.9789726375089 8.45494586667155 10.7149544447643 6.00733109978337 8.45494600287951 10.7149543720477 6.0073311092401 8.62494600287951</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface>
          <gml:name>Roof 1</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69227_652_333360_102720">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69227_652_333360_102720_0">
                      <gml:posList>10.8222561090575 6.02482974132821 8.85491583744193 10.8016364794322 6.03444488483946 8.85491564617534 10.7208288385256 5.86115270274922 8.70494590082275 10.0799776476801 6.15998654912105 8.70494561167759 10.0732441385545 6.14554652245525 8.69244897460679 10.7347149579192 5.83709753020005 8.69244946195073 10.8222561090575 6.02482974132821 8.85491583744193</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69228_1908_757076_233223">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69228_1908_757076_233223_0">
                      <gml:posList>10.1607852748879 6.33327870183421 8.85491551425347 10.0799776476801 6.15998654912105 8.70494561167759 10.7208288385256 5.86115270274922 8.70494590082275 10.8016364794322 6.03444488483946 8.85491564617534 10.1607852748879 6.33327870183421 8.85491551425347</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 10</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69229_605_783256_230125">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69229_605_783256_230125_0">
                      <gml:posList>10.7656685828994 6.11608760265913 8.45494597135541 10.774120793816 6.13421337394904 8.45494596610148 10.7741207210994 6.13421338340576 8.62494596610148 10.7149543720477 6.0073311092401 8.62494600287951 10.7149544447643 6.00733109978337 8.45494600287951 10.7234066542158 6.02545687175651 8.45494599762558 10.7234066202291 6.02545687322147 8.55494599762558 10.7241266344147 6.02700096338779 8.56788694943314 10.7262376211227 6.03152799362934 8.57994599586581 10.7295957200976 6.03872945431641 8.59030133283772 10.7339720824438 6.04811457780416 8.59824726124725 10.7390684662996 6.05904378343656 8.60324227920455 10.7445375615275 6.07077226391111 8.60494598449048 10.7500066583257 6.08250074266572 8.60324227240533 10.7551030467856 6.09342994325547 8.59824724811216 10.7594794164558 6.10281505872155 8.59030131426191 10.7628375249756 6.11001650895456 8.57994597311517 10.7649485227989 6.11454352702209 8.5678869240581 10.7656685489127 6.1160876041241 8.55494597135541 10.7656685828994 6.11608760265913 8.45494597135541</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
          <bldg:opening>
            <bldg:Door>
              <gml:name>Door 10.1</gml:name>
              <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
              <bldg:lod3MultiSurface>
                <gml:MultiSurface>
                  <gml:surfaceMember>
                    <gml:Polygon gml:id="PolyID69230_1544_262083_153863">
                      <gml:exterior>
                        <gml:LinearRing gml:id="PolyID69230_1544_262083_153863_0">
                          <gml:posList>10.7234066542158 6.02545687175651 8.45494599762558 10.7656685828994 6.11608760265913 8.45494597135541 10.7656685489127 6.1160876041241 8.55494597135541 10.7649485227989 6.11454352702209 8.5678869240581 10.7628375249756 6.11001650895456 8.57994597311517 10.7594794164558 6.10281505872155 8.59030131426191 10.7551030467856 6.09342994325547 8.59824724811216 10.7500066583257 6.08250074266572 8.60324227240533 10.7445375615275 6.07077226391111 8.60494598449048 10.7390684662996 6.05904378343656 8.60324227920455 10.7339720824438 6.04811457780416 8.59824726124725 10.7295957200976 6.03872945431641 8.59030133283772 10.7262376211227 6.03152799362934 8.57994599586581 10.7241266344147 6.02700096338779 8.56788694943314 10.7234066202291 6.02545687322147 8.55494599762558 10.7234066542158 6.02545687175651 8.45494599762558</gml:posList>
                        </gml:LinearRing>
                      </gml:exterior>
                    </gml:Polygon>
                  </gml:surfaceMember>
                </gml:MultiSurface>
              </bldg:lod3MultiSurface>
            </bldg:Door>
          </bldg:opening>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface>
          <gml:name>Roof 2</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69231_358_160284_85571">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69231_358_160284_85571_0">
                      <gml:posList>10.1607852748879 6.33327870183421 8.85491551425347 10.8016364794322 6.03444488483946 8.85491564617534 10.8898762036283 6.22367487657448 8.70494579574227 10.2490249853852 6.52250866419215 8.70494582104368 10.1607852748879 6.33327870183421 8.85491551425347</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69232_736_849584_241351">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69232_736_849584_241351_0">
                      <gml:posList>10.2558900109489 6.53723070398215 8.69327821767228 10.2490249853852 6.52250866419215 8.70494582104368 10.8898762036283 6.22367487657448 8.70494579574227 10.8016364794322 6.03444488483946 8.85491564617534 10.8222561090575 6.02482974132821 8.85491583744193 10.917360858664 6.22878177252456 8.6932783768555 10.2558900109489 6.53723070398215 8.69327821767228</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface>
          <gml:name>Wall 11</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69233_1537_414373_329290">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69233_1537_414373_329290_0">
                      <gml:posList>10.774120793816 6.13421337394904 8.45494596610148 10.8349358050333 6.10585491167457 8.45494582989353 10.8349357323167 6.10585492113129 8.62494582989352 10.7741207210994 6.13421338340576 8.62494596610148 10.774120793816 6.13421337394904 8.45494596610148</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface>
          <gml:name>Roof 3</gml:name>
          <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
          <bldg:lod3MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69234_724_72786_56725">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69234_724_72786_56725_0">
                      <gml:posList>10.1607852748879 6.33327870183421 8.85491551425347 10.0316088019408 6.29287911537808 8.7049456034536 10.0799776476801 6.15998654912105 8.70494561167759 10.1607852748879 6.33327870183421 8.85491551425347</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69235_392_555907_74487">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69235_392_555907_74487_0">
                      <gml:posList>10.0208448312489 6.28951271314621 8.69244896537074 10.0316088019408 6.29287911537808 8.7049456034536 10.1161322822779 6.47413976864175 8.70494550825457 10.1126582914302 6.48509872192144 8.69327788101129 10.0208448312489 6.28951271314621 8.69244896537074</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69236_502_525849_243785">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69236_502_525849_243785_0">
                      <gml:posList>10.0316088019408 6.29287911537808 8.7049456034536 10.0208448312489 6.28951271314621 8.69244896537074 10.0732441385545 6.14554652245525 8.69244897460679 10.0799776476801 6.15998654912105 8.70494561167759 10.0316088019408 6.29287911537808 8.7049456034536</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69237_636_514520_280648">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69237_636_514520_280648_0">
                      <gml:posList>10.0316088019408 6.29287911537808 8.7049456034536 10.1607852748879 6.33327870183421 8.85491551425347 10.1161322822779 6.47413976864175 8.70494550825457 10.0316088019408 6.29287911537808 8.7049456034536</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69238_843_626353_75611">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69238_843_626353_75611_0">
                      <gml:posList>10.1126582914302 6.48509872192144 8.69327788101129 10.1161322822779 6.47413976864175 8.70494550825457 10.2490249853852 6.52250866419215 8.70494582104368 10.2558900109489 6.53723070398215 8.69327821767228 10.1126582914302 6.48509872192144 8.69327788101129</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="PolyID69239_1458_472575_159548">
                  <gml:exterior>
                    <gml:LinearRing gml:id="PolyID69239_1458_472575_159548_0">
                      <gml:posList>10.1161322822779 6.47413976864175 8.70494550825457 10.1607852748879 6.33327870183421 8.85491551425347 10.2490249853852 6.52250866419215 8.70494582104368 10.1161322822779 6.47413976864175 8.70494550825457</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod3MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
    </bldg:Building>
  </cityObjectMember>
<cityObjectMember>
    <brid:Bridge gml:id="GMLID_BUI205585_1385_1373">
      <gml:name>Bridge KIT 4</gml:name>
      <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
      <brid:lod3MultiSurface>
        <gml:MultiSurface>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33472_1613_707495_49371">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33472_1613_707495_49371_0">
                  <gml:posList>12.4610065272718 3.27948462024282 8.42451623042048 12.4608877835914 4.379281903167 8.4455094766505 12.4603310302295 4.37890027521485 8.46549787262475 12.4604497662159 3.27910303463571 8.44450488945687 12.4610065272718 3.27948462024282 8.42451623042048</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33473_1859_44578_334663">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33473_1859_44578_334663_0">
                  <gml:posList>12.2804010868904 4.37897668640225 8.46048749811746 12.280957947983 4.37935831534022 8.44049905111472 12.2810765839327 3.27956103143021 8.41950585591319 12.2805198228768 3.27917944582311 8.43949451494958 12.2804010868904 4.37897668640225 8.46048749811746</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33474_1475_727931_350004">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33474_1475_727931_350004_0">
                  <gml:posList>12.4604497662159 3.27910303463571 8.44450488945687 12.4603310302295 4.37890027521485 8.46549787262475 12.2804010868904 4.37897668640225 8.46048749811746 12.2805198228768 3.27917944582311 8.43949451494958 12.4604497662159 3.27910303463571 8.44450488945687</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33475_994_462091_365906">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33475_994_462091_365906_0">
                  <gml:posList>12.4603310302295 4.37890027521485 8.46549787262475 12.4608877835914 4.379281903167 8.4455094766505 12.280957947983 4.37935831534022 8.44049905111472 12.2804010868904 4.37897668640225 8.46048749811746 12.4603310302295 4.37890027521485 8.46549787262475</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33476_1770_890717_401053">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33476_1770_890717_401053_0">
                  <gml:posList>12.4608877835914 4.379281903167 8.4455094766505 12.4610065272718 3.27948462024282 8.42451623042048 12.2810765839327 3.27956103143021 8.41950585591319 12.280957947983 4.37935831534022 8.44049905111472 12.4608877835914 4.379281903167 8.4455094766505</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33477_583_629528_356488">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33477_583_629528_356488_0">
                  <gml:posList>12.4610065272718 3.27948462024282 8.42451623042048 12.4604497662159 3.27910303463571 8.44450488945687 12.2805198228768 3.27917944582311 8.43949451494958 12.2810765839327 3.27956103143021 8.41950585591319 12.4610065272718 3.27948462024282 8.42451623042048</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33478_68_319734_221843">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33478_68_319734_221843_0">
                  <gml:posList>12.5003154452742 4.37888361705613 8.46661129086182 12.4997586842183 4.37850203144904 8.48659994989821 12.4998773201679 3.27870474753903 8.46560675469668 12.5004340812238 3.27908633314613 8.44561809566029 12.5003154452742 4.37888361705613 8.46661129086182</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33479_575_480822_385099">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33479_575_480822_385099_0">
                  <gml:posList>12.4403387282386 4.3789090726797 8.46494111235767 12.4397819671828 4.37852748707261 8.48492977139406 12.4997586842183 4.37850203144904 8.48659994989821 12.5003154452742 4.37888361705613 8.46661129086182 12.4403387282386 4.3789090726797 8.46494111235767</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33480_1263_489591_393600">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33480_1263_489591_393600_0">
                  <gml:posList>12.440457612559 3.27911136420799 8.4439481548241 12.4399007514664 3.27872973527003 8.46393660182684 12.4397819671828 4.37852748707261 8.48492977139406 12.4403387282386 4.3789090726797 8.46494111235767 12.440457612559 3.27911136420799 8.4439481548241</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33481_1283_212547_327530">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33481_1283_212547_327530_0">
                  <gml:posList>12.5004340812238 3.27908633314613 8.44561809566029 12.4998773201679 3.27870474753903 8.46560675469668 12.4399007514664 3.27872973527003 8.46393660182684 12.440457612559 3.27911136420799 8.4439481548241 12.5004340812238 3.27908633314613 8.44561809566029</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33482_1679_413735_415448">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33482_1679_413735_415448_0">
                  <gml:posList>12.4997586842183 4.37850203144904 8.48659994989821 12.4397819671828 4.37852748707261 8.48492977139406 12.4399007514664 3.27872973527003 8.46393660182684 12.4998773201679 3.27870474753903 8.46560675469668 12.4997586842183 4.37850203144904 8.48659994989821</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33483_879_70299_85222">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33483_879_70299_85222_0">
                  <gml:posList>12.4403387282386 4.3789090726797 8.46494111235767 12.5003154452742 4.37888361705613 8.46661129086182 12.5004340812238 3.27908633314613 8.44561809566029 12.440457612559 3.27911136420799 8.4439481548241 12.4403387282386 4.3789090726797 8.46494111235767</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33484_1067_465949_199204">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33484_1067_465949_199204_0">
                  <gml:posList>12.3003932405473 4.37896835682998 8.46104423275024 12.2998365872222 4.3785867722087 8.48103284075815 12.2999552637751 3.2787890194203 8.46003972221941 12.3005121248677 3.27917064835827 8.44005127521667 12.3003932405473 4.37896835682998 8.46104423275024</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33485_81_71871_130969">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33485_81_71871_130969_0">
                  <gml:posList>12.2404167718825 4.37899338789184 8.45937429191405 12.2398600185206 4.3786117599397 8.4793626878883 12.2998365872222 4.3785867722087 8.48103284075815 12.3003932405473 4.37896835682998 8.46104423275024 12.2404167718825 4.37899338789184 8.45937429191405</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33486_951_147502_197524">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33486_951_147502_197524_0">
                  <gml:posList>12.2405354078321 3.27919610398184 8.43838109671252 12.239978754507 3.27881451936056 8.45836970472042 12.2398600185206 4.3786117599397 8.4793626878883 12.2404167718825 4.37899338789184 8.45937429191405 12.2405354078321 3.27919610398184 8.43838109671252</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33487_1940_835163_9981">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33487_1940_835163_9981_0">
                  <gml:posList>12.3005121248677 3.27917064835827 8.44005127521667 12.2999552637751 3.2787890194203 8.46003972221941 12.239978754507 3.27881451936056 8.45836970472042 12.2405354078321 3.27919610398184 8.43838109671252 12.3005121248677 3.27917064835827 8.44005127521667</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33488_394_150924_409333">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33488_394_150924_409333_0">
                  <gml:posList>12.2998365872222 4.3785867722087 8.48103284075815 12.2398600185206 4.3786117599397 8.4793626878883 12.239978754507 3.27881451936056 8.45836970472042 12.2999552637751 3.2787890194203 8.46003972221941 12.2998365872222 4.3785867722087 8.48103284075815</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33489_1350_40106_59947">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33489_1350_40106_59947_0">
                  <gml:posList>12.2404167718825 4.37899338789184 8.45937429191405 12.3003932405473 4.37896835682998 8.46104423275024 12.3005121248677 3.27917064835827 8.44005127521667 12.2405354078321 3.27919610398184 8.43838109671252 12.2404167718825 4.37899338789184 8.45937429191405</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33490_1781_47225_100338">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33490_1781_47225_100338_0">
                  <gml:posList>12.5104302322193 3.27908193441371 8.44589647579383 12.5103114885389 4.37887921733789 8.46688972202384 12.5083628661011 4.37754386030011 8.53684970174343 12.5084815426539 3.27774610751171 8.5158565832047 12.5104302322193 3.27908193441371 8.44589647579383</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33491_642_604424_159781">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33491_642_604424_159781_0">
                  <gml:posList>12.4983667557089 4.37754779015414 8.53657139827269 12.5003154452742 4.37888361705613 8.46661129086182 12.5004340812238 3.27908633314613 8.44561809566029 12.4984854993893 3.27775050722996 8.51557815204267 12.4983667557089 4.37754779015414 8.53657139827269</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33492_829_244957_62068">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33492_829_244957_62068_0">
                  <gml:posList>12.5084815426539 3.27774610751171 8.5158565832047 12.5083628661011 4.37754386030011 8.53684970174343 12.4983667557089 4.37754779015414 8.53657139827269 12.4984854993893 3.27775050722996 8.51557815204267 12.5084815426539 3.27774610751171 8.5158565832047</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33493_750_479082_48933">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33493_750_479082_48933_0">
                  <gml:posList>12.5083628661011 4.37754386030011 8.53684970174343 12.5103114885389 4.37887921733789 8.46688972202384 12.5003154452742 4.37888361705613 8.46661129086182 12.4983667557089 4.37754779015414 8.53657139827269 12.5083628661011 4.37754386030011 8.53684970174343</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33494_324_720319_84991">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33494_324_720319_84991_0">
                  <gml:posList>12.5103114885389 4.37887921733789 8.46688972202384 12.5104302322193 3.27908193441371 8.44589647579383 12.5004340812238 3.27908633314613 8.44561809566029 12.5003154452742 4.37888361705613 8.46661129086182 12.5103114885389 4.37887921733789 8.46688972202384</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33495_1216_43466_237440">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33495_1216_43466_237440_0">
                  <gml:posList>12.5104302322193 3.27908193441371 8.44589647579383 12.5084815426539 3.27774610751171 8.5158565832047 12.4984854993893 3.27775050722996 8.51557815204267 12.5004340812238 3.27908633314613 8.44561809566029 12.5104302322193 3.27908193441371 8.44589647579383</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33496_188_482587_401862">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33496_188_482587_401862_0">
                  <gml:posList>12.2405354078321 3.27919610398184 8.43838109671252 12.2404167718825 4.37899338789184 8.45937429191405 12.2384679416772 4.37765798653738 8.52933411062847 12.2385868931251 3.27786074792988 8.50834102540362 12.2405354078321 3.27919610398184 8.43838109671252</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33497_1470_19073_397883">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33497_1470_19073_397883_0">
                  <gml:posList>12.2284718907185 4.37766242860067 8.52905594252858 12.2304206208871 4.37899778662427 8.4590959117805 12.2305392568367 3.27920050271426 8.43810271657897 12.2285907827329 3.27786467778391 8.50806272193288 12.2284718907185 4.37766242860067 8.52905594252858</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33498_1373_12399_89470">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33498_1373_12399_89470_0">
                  <gml:posList>12.2385868931251 3.27786074792988 8.50834102540362 12.2384679416772 4.37765798653738 8.52933411062847 12.2284718907185 4.37766242860067 8.52905594252858 12.2285907827329 3.27786467778391 8.50806272193288 12.2385868931251 3.27786074792988 8.50834102540362</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33499_1087_415448_124975">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33499_1087_415448_124975_0">
                  <gml:posList>12.2384679416772 4.37765798653738 8.52933411062847 12.2404167718825 4.37899338789184 8.45937429191405 12.2304206208871 4.37899778662427 8.4590959117805 12.2284718907185 4.37766242860067 8.52905594252858 12.2384679416772 4.37765798653738 8.52933411062847</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33500_802_385266_297956">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33500_802_385266_297956_0">
                  <gml:posList>12.2404167718825 4.37899338789184 8.45937429191405 12.2405354078321 3.27919610398184 8.43838109671252 12.2305392568367 3.27920050271426 8.43810271657897 12.2304206208871 4.37899778662427 8.4590959117805 12.2404167718825 4.37899338789184 8.45937429191405</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
          <gml:surfaceMember>
            <gml:Polygon gml:id="PolyID33501_724_290162_416285">
              <gml:exterior>
                <gml:LinearRing gml:id="PolyID33501_724_290162_416285_0">
                  <gml:posList>12.2405354078321 3.27919610398184 8.43838109671252 12.2385868931251 3.27786074792988 8.50834102540362 12.2285907827329 3.27786467778391 8.50806272193288 12.2305392568367 3.27920050271426 8.43810271657897 12.2405354078321 3.27919610398184 8.43838109671252</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </brid:lod3MultiSurface>
    </brid:Bridge>
  </cityObjectMember>
<cityObjectMember>
    <veg:SolitaryVegetationObject gml:id="GMLID_SO015374_872_14131">
      <gml:name>Forest tree 2</gml:name>
      <relativeToTerrain>entirelyAboveTerrain</relativeToTerrain>
      <veg:class>1060</veg:class>
      <veg:species>1640</veg:species>
      <veg:lod3ImplicitRepresentation>
        <ImplicitGeometry>
          <transformationMatrix>1.0 0.0 0.0 0.695016027750339 0.0 1.0 0.0 6.82842600075878 0.0 0.0 1.0 8.950045663907 0.0 0.0 0.0 1.0</transformationMatrix>
          <relativeGMLGeometry xlink:href="#Mult_SO045475_3243_790864"/>
          <referencePoint>
            <gml:Point>
              <gml:pos srsDimension="3">0.0 0.0 0.0</gml:pos>
            </gml:Point>
          </referencePoint>
        </ImplicitGeometry>
      </veg:lod3ImplicitRepresentation>
    </veg:SolitaryVegetationObject>
  </cityObjectMember>
<cityObjectMember>
    <grp:CityObjectGroup gml:id="UUID_f488e8ce-b953-4b35-a3fe-a394fb203868">
      <gml:description>CityObjectGroup for grouping the trees to a forest</gml:description>
      <gml:name>Forest KIT</gml:name>
      <grp:groupMember xlink:href="#GMLID_SO0200811_525_4986"/>
      <grp:groupMember xlink:href="#GMLID_SO015374_872_14131"/>
      <grp:groupMember xlink:href="#GMLID_SO0158953_776_12462"/>
      <grp:groupMember xlink:href="#GMLID_SO0321978_1466_7441"/>
      <grp:groupMember xlink:href="#GMLID_SO0286258_965_2893"/>
      <grp:groupMember xlink:href="#GMLID_SO0410128_214_10515"/>
      <grp:groupMember xlink:href="#GMLID_SO0283142_1335_8999"/>
      <grp:groupMember xlink:href="#GMLID_SO086970_1053_8814"/>
      <grp:groupMember xlink:href="#GMLID_SO0107241_3793_12555"/>
      <grp:groupMember xlink:href="#GMLID_SO081792_2341_11961"/>
      <grp:groupMember xlink:href="#GMLID_SO0260143_1214_1315"/>
      <grp:groupMember xlink:href="#GMLID_SO0400934_2926_3381"/>
      <grp:groupMember xlink:href="#GMLID_SO0124800_3522_13577"/>
      <grp:groupMember xlink:href="#GMLID_SO0443764_1492_9667"/>
    </grp:CityObjectGroup>
  </cityObjectMember>
</CityModel>
