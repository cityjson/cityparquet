<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment from Japan's PLATEAU 3D City Models (Tokyo, Chiyoda-ku;
  13101_chiyoda-ku_pref_2025_citygml_1_op), module `urf` (urban planning
  districts, i-UR Urban Function ADE), tile 533946_urf_6697_08_op.gml.
  Licence: PLATEAU site policy, compatible with CC BY 4.0 (MLIT Japan). The
  whole tile is lib/citygml-rs/crates/citygml2/tests/data/plateau/.

  Root CityModel + gml:Envelope + the tile's FIRST THREE cityObjectMembers,
  verbatim.

  Every member is an `urf:` ADE class, which the reader does not map. A
  reader that silently skips unmapped members therefore reports 0 objects for
  a document that plainly holds three, in a fraction of the time a real read
  would take. That silent zero is exactly what the readbench `citygml`
  runner's skipped-member guard refuses.
-->
<core:CityModel xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:urf="https://www.geospatial.jp/iur/urf/3.2" xmlns:uro="https://www.geospatial.jp/iur/uro/3.2" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/profiles/base/2.0 http://schemas.opengis.net/citygml/profiles/base/2.0/CityGML.xsd https://www.geospatial.jp/iur/uro/3.2 ../../schemas/iur/uro/3.2/urbanObject.xsd https://www.geospatial.jp/iur/urf/3.2 ../../schemas/iur/urf/3.2/urbanFunction.xsd">
  <gml:boundedBy>
    <gml:Envelope srsDimension="3" srsName="http://www.opengis.net/def/crs/EPSG/0/6697">
      <gml:lowerCorner>35.64269745139441 139.7177820393545 0</gml:lowerCorner>
      <gml:upperCorner>35.76960503338157 139.88545955359368 0</gml:upperCorner>
    </gml:Envelope>
  </gml:boundedBy>
  <core:cityObjectMember>
    <urf:FirePreventionDistrict gml:id="urf_aa9fab84-6e72-4224-9a2e-ef5cf5452497"><core:creationDate>2026-03-13</core:creationDate>
      <urf:function codeSpace="../../codelists/Common_districtsAndZonesType.xml">24</urf:function>
      <urf:usage codeSpace="../../codelists/FirePreventionDistrict_usage.xml">1</urf:usage>
      <urf:validFrom>2004-06-24</urf:validFrom>
      <urf:validFromType codeSpace="../../codelists/Common_validType.xml">1</urf:validFromType>
      <urf:custodian>江東区</urf:custodian>
      <urf:notificationNumber>江東区告示第157号</urf:notificationNumber>
      <urf:finalNotificationDate>2022-12-19</urf:finalNotificationDate>
      <urf:finalNotificationNumber>江東区告示第386号</urf:finalNotificationNumber>
      <urf:prefecture codeSpace="../../codelists/Common_localPublicAuthorities.xml">13</urf:prefecture>
      <urf:city codeSpace="../../codelists/Common_localPublicAuthorities.xml">13100</urf:city>
      <urf:lod1MultiSurface><gml:MultiSurface srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3"><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>35.66762592777675 139.80044061128262 0 35.66762589161923 139.8004402357374 0 35.66760930402866 139.8001361935991 0 35.66760759179212 139.80010486818784 0 35.66760596103718 139.8000748682499 0 35.66760477447665 139.80002141772303 0 35.66760323195138 139.79995207232315 0 35.667618030563546 139.79958935250264 0 35.66761873777855 139.79957197703095 0 35.66650244304334 139.79971071746144 0 35.66653168261897 139.80006411344016 0 35.66679876823209 139.80002967237598 0 35.66680238456594 139.8000688833598 0 35.66683394886863 139.80045613513764 0 35.66684064148347 139.80053845656167 0 35.666894686017464 139.80053167443938 0 35.66693841917709 139.80052615574678 0 35.66715299287117 139.8004990830222 0 35.667190623937735 139.80049434000065 0 35.66720865079892 139.80049207926947 0 35.66736856713659 139.80047223065918 0 35.66753469372361 139.80045161725778 0 35.66757150461726 139.8004471727663 0 35.66762592777675 139.80044061128262 0</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </urf:lod1MultiSurface>
      <urf:dataQualityAttribute><uro:DataQualityAttribute><uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">400</uro:geometrySrcDescLod1>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">400</uro:thematicSrcDesc>
        </uro:DataQualityAttribute>
      </urf:dataQualityAttribute>
      <urf:location>江東区古石場二丁目</urf:location>
    </urf:FirePreventionDistrict>
  </core:cityObjectMember>
  <core:cityObjectMember>
    <urf:FirePreventionDistrict gml:id="urf_7f6cdcce-77bc-4fd7-a815-b8323fde7b98"><core:creationDate>2026-03-13</core:creationDate>
      <urf:function codeSpace="../../codelists/Common_districtsAndZonesType.xml">24</urf:function>
      <urf:usage codeSpace="../../codelists/FirePreventionDistrict_usage.xml">1</urf:usage>
      <urf:validFrom>1999-04-13</urf:validFrom>
      <urf:validFromType codeSpace="../../codelists/Common_validType.xml">3</urf:validFromType>
      <urf:custodian>葛飾区</urf:custodian>
      <urf:notificationNumber>葛飾区告示第94号</urf:notificationNumber>
      <urf:finalNotificationDate>1999-04-01</urf:finalNotificationDate>
      <urf:finalNotificationNumber>葛飾区告示第184号</urf:finalNotificationNumber>
      <urf:prefecture codeSpace="../../codelists/Common_localPublicAuthorities.xml">13</urf:prefecture>
      <urf:city codeSpace="../../codelists/Common_localPublicAuthorities.xml">13100</urf:city>
      <urf:lod1MultiSurface><gml:MultiSurface srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3"><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>35.73462335404084 139.8725999570792 0 35.73465416198863 139.8726571935692 0 35.734648956243014 139.8724786506307 0 35.73464655833076 139.87239632189278 0 35.73458622836112 139.87130344928082 0 35.73456762845963 139.8713185528014 0 35.73442867457159 139.87143148100597 0 35.73442834999666 139.87143174617378 0 35.73401065121776 139.871764867847 0 35.73393744132108 139.87182478411967 0 35.733971350833336 139.8724391289104 0 35.73397486305876 139.87255939839326 0 35.73397513934567 139.87256901641246 0 35.73397613760647 139.87260355284346 0 35.7340119566308 139.8726071522628 0 35.73413103795494 139.87257294004365 0 35.73429790594118 139.87249453081157 0 35.73436741255115 139.87245929909457 0 35.734479200070076 139.87240264106916 0 35.73451075192437 139.8723899431007 0 35.73455723397852 139.87247708013612 0 35.734558810415365 139.8724799994591 0 35.734578556362614 139.87251670107324 0 35.73462335404084 139.8725999570792 0</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </urf:lod1MultiSurface>
      <urf:dataQualityAttribute><uro:DataQualityAttribute><uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">400</uro:geometrySrcDescLod1>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">400</uro:thematicSrcDesc>
        </uro:DataQualityAttribute>
      </urf:dataQualityAttribute>
      <urf:location>葛飾区奥戸九丁目</urf:location>
    </urf:FirePreventionDistrict>
  </core:cityObjectMember>
  <core:cityObjectMember>
    <urf:FirePreventionDistrict gml:id="urf_d60fbdd6-7038-4e37-b4c6-8c31be142f3d"><core:creationDate>2026-03-13</core:creationDate>
      <urf:function codeSpace="../../codelists/Common_districtsAndZonesType.xml">24</urf:function>
      <urf:usage codeSpace="../../codelists/FirePreventionDistrict_usage.xml">1</urf:usage>
      <urf:validFrom>2004-06-24</urf:validFrom>
      <urf:validFromType codeSpace="../../codelists/Common_validType.xml">1</urf:validFromType>
      <urf:custodian>北区</urf:custodian>
      <urf:notificationNumber>北区告示第238号</urf:notificationNumber>
      <urf:finalNotificationDate>2004-06-02</urf:finalNotificationDate>
      <urf:finalNotificationNumber>北区告示第122号</urf:finalNotificationNumber>
      <urf:prefecture codeSpace="../../codelists/Common_localPublicAuthorities.xml">13</urf:prefecture>
      <urf:city codeSpace="../../codelists/Common_localPublicAuthorities.xml">13100</urf:city>
      <urf:lod1MultiSurface><gml:MultiSurface srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3"><gml:surfaceMember><gml:Polygon><gml:exterior><gml:LinearRing><gml:posList>35.738109133671806 139.7600218035991 0 35.73825312631476 139.75980007260054 0 35.73825622431936 139.7597956806677 0 35.73826733753677 139.75977997145478 0 35.73827691222753 139.7597688517397 0 35.73828862162526 139.7597552425407 0 35.73830334818524 139.7597377058111 0 35.7383131388676 139.759726044161 0 35.738335990902854 139.75970063938084 0 35.73834770966499 139.75968761610218 0 35.738374120191935 139.75965913455292 0 35.73838356056066 139.75964947427755 0 35.73839334322966 139.75963945990375 0 35.7384581243036 139.75958080523327 0 35.73850536554535 139.75953802047593 0 35.73859688332083 139.75945379128495 0 35.7386209791979 139.75942841835442 0 35.738629014162505 139.75941996438996 0 35.73864392949 139.75940166450735 0 35.73865226679168 139.7593864994586 0 35.73865540904041 139.75938079182055 0 35.73865726274151 139.75937573765896 0 35.73866240093727 139.7593617695779 0 35.738666677738024 139.75933951052812 0 35.738670287935804 139.75930326665068 0 35.738670310720494 139.75926705925937 0 35.73866833632339 139.75925174896375 0 35.738666878151975 139.75924041823725 0 35.73865993304542 139.7592036645218 0 35.73865828956174 139.75919879048857 0 35.73865291664475 139.75918287528532 0 35.7386421848756 139.75915924818804 0 35.73863143903554 139.7591420444603 0 35.73862623302686 139.75913370225177 0 35.73856750048034 139.75906108781476 0 35.73847514569672 139.7589425353109 0 35.73837601611966 139.75881520015474 0 35.73826837078804 139.75867698346678 0 35.73819366935601 139.75858105725348 0 35.73791908474933 139.7590575109681 0 35.73785617463572 139.75916794823388 0 35.737790140073464 139.7592838828342 0 35.73773689432516 139.75938284619718 0 35.73771387478933 139.7594289359726 0 35.737692565734385 139.7594716190324 0 35.73764852587397 139.75957568132068 0 35.73761689082343 139.75966852175316 0 35.737634219852325 139.75967641050494 0 35.73764366004937 139.7596811888613 0 35.737677832316216 139.75969851460306 0 35.73772043694736 139.75972339459707 0 35.73772400772304 139.7597256466438 0 35.73772743422656 139.75972781037905 0 35.737759605177295 139.75974467366677 0 35.737852255743235 139.759802751785 0 35.73779545108154 139.7599195949955 0 35.73768214748212 139.76015262836154 0 35.737679671827955 139.76015771617915 0 35.73791845410151 139.7604583200844 0 35.73806381044431 139.76064131262518 0 35.73816406510751 139.76051553020804 0 35.738323034812595 139.76031608548828 0 35.738230143838166 139.76018828988398 0 35.738109133671806 139.7600218035991 0</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </urf:lod1MultiSurface>
      <urf:dataQualityAttribute><uro:DataQualityAttribute><uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">400</uro:geometrySrcDescLod1>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">400</uro:thematicSrcDesc>
        </uro:DataQualityAttribute>
      </urf:dataQualityAttribute>
      <urf:location>北区田端六丁目</urf:location>
    </urf:FirePreventionDistrict>
  </core:cityObjectMember>
</core:CityModel>
