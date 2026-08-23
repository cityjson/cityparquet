<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment: the first 3 buildings extracted verbatim from the SAVeNoW
  LoD2 building models (Ingolstadt, Germany), CityGML 2.0.
  Source: https://github.com/savenow/lod3-road-space-models
    models/building/lod2/combined/citygml/lod2_building_models.gml.gz
    @ commit fdddf41d62c49282bf319b79b9f0a7b0ce1152b2
  Licence: CC BY 4.0 (models/building/lod2/LICENSE in the source repo).
  Root CityModel + envelope + 3 cityObjectMembers, copied unmodified; no
  geometry or attribute value was changed. Regenerate with the same slice.
-->
<core:CityModel xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:sch="http://www.ascc.net/xml/schematron" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:tex="http://www.opengis.net/citygml/texturedsurface/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:smil20="http://www.w3.org/2001/SMIL20/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:smil20lang="http://www.w3.org/2001/SMIL20/Language" xmlns:pbase="http://www.opengis.net/citygml/profiles/base/2.0" xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/2.0" gml:id="fme-gen-56f7dbf3-f8b9-4854-9670-6c358b7ca3de">
<gml:boundedBy>
<gml:Envelope srsName="EPSG:25832" srsDimension="3">
<gml:lowerCorner>675864.55 5401979.025 361.22</gml:lowerCorner>
<gml:upperCorner>680047.72 5406051.482 437.95</gml:upperCorner>
</gml:Envelope>
</gml:boundedBy>
<core:cityObjectMember>
<bldg:Building gml:id="DEBY_LOD2_51985910">
<core:creationDate>2024-02-23</core:creationDate>
<core:externalReference>
<core:informationSystem>http://repository.gdi-de.org/schemas/adv/citygml/fdv/art.htm#_9100</core:informationSystem>
<core:externalObject>
<core:name>DEBYvAAAAABHDJAJ</core:name>
</core:externalObject>
</core:externalReference>
<gen:stringAttribute name="citygml_function">
<gen:value>51009_1610</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleBodenhoehe">
<gen:value>1100</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleDachhoehe">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleLage">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Gemeindeschluessel">
<gen:value>09161000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Geometrietyp2DReferenz">
<gen:value>3000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Grundrissaktualitaet">
<gen:value>2024-01-12</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="HoeheDach">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="HoeheGrund">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Methode">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="NiedrigsteTraufeDesGebaeudes">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<bldg:roofType>1000</bldg:roofType>
<bldg:measuredHeight uom="urn:adv:uom:m">3.448</bldg:measuredHeight>
<bldg:lod2Solid>
<gml:Solid srsName="EPSG:25832" srsDimension="3">
<gml:exterior>
<gml:CompositeSurface>
<gml:surfaceMember xlink:href="#DEBY_LOD2_51985910_45d3ac32-8468-4ef2-b3de-3eeb374d8659_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_51985910_6466ad97-8b3b-4458-a1fd-fb0387cce7a1_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_51985910_ae3797a1-9524-4dd2-9934-0a85dd56218e_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_51985910_bf5c89dd-827b-4245-98b0-117deddad43c_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_51985910_20328cc6-7e13-4f8d-abc1-4a3f1d8ee240_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_51985910_5f72d41c-adeb-4e90-8584-08a37c43fdb9_poly"/>
</gml:CompositeSurface>
</gml:exterior>
</gml:Solid>
</bldg:lod2Solid>
<bldg:boundedBy>
<bldg:GroundSurface gml:id="DEBY_LOD2_51985910_45d3ac32-8468-4ef2-b3de-3eeb374d8659">
<gen:stringAttribute name="Flaeche">
<gen:value>31.931</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_51985910_45d3ac32-8468-4ef2-b3de-3eeb374d8659_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676871.372 5403206.194 367.83 676874.803 5403207.569 367.83 676878.017 5403199.56 367.83 676874.578 5403198.183 367.83 676871.372 5403206.194 367.83</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:GroundSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_51985910_6466ad97-8b3b-4458-a1fd-fb0387cce7a1">
<gen:stringAttribute name="Flaeche">
<gen:value>12.745</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>3.448</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_51985910_6466ad97-8b3b-4458-a1fd-fb0387cce7a1_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676871.372 5403206.194 367.83 676871.372 5403206.194 371.278 676874.803 5403207.569 371.278 676874.803 5403207.569 367.83 676871.372 5403206.194 367.83</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_51985910_ae3797a1-9524-4dd2-9934-0a85dd56218e">
<gen:stringAttribute name="Flaeche">
<gen:value>29.752</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>3.448</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_51985910_ae3797a1-9524-4dd2-9934-0a85dd56218e_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676874.578 5403198.183 367.83 676874.578 5403198.183 371.278 676871.372 5403206.194 371.278 676871.372 5403206.194 367.83 676874.578 5403198.183 367.83</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:RoofSurface gml:id="DEBY_LOD2_51985910_bf5c89dd-827b-4245-98b0-117deddad43c">
<gen:stringAttribute name="Dachneigung">
<gen:value>90.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Dachorientierung">
<gen:value>-1.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Flaeche">
<gen:value>31.931</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>3.448</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>3.448</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_51985910_bf5c89dd-827b-4245-98b0-117deddad43c_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676874.578 5403198.183 371.278 676878.017 5403199.56 371.278 676874.803 5403207.569 371.278 676871.372 5403206.194 371.278 676874.578 5403198.183 371.278</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:RoofSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_51985910_20328cc6-7e13-4f8d-abc1-4a3f1d8ee240">
<gen:stringAttribute name="Flaeche">
<gen:value>12.773</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>3.448</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_51985910_20328cc6-7e13-4f8d-abc1-4a3f1d8ee240_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676878.017 5403199.56 367.83 676878.017 5403199.56 371.278 676874.578 5403198.183 371.278 676874.578 5403198.183 367.83 676878.017 5403199.56 367.83</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_51985910_5f72d41c-adeb-4e90-8584-08a37c43fdb9">
<gen:stringAttribute name="Flaeche">
<gen:value>29.756</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>3.448</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.278</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.830</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_51985910_5f72d41c-adeb-4e90-8584-08a37c43fdb9_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676874.803 5403207.569 367.83 676874.803 5403207.569 371.278 676878.017 5403199.56 371.278 676878.017 5403199.56 367.83 676874.803 5403207.569 367.83</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
</bldg:Building>
</core:cityObjectMember>
<core:cityObjectMember>
<bldg:Building gml:id="DEBY_LOD2_107777354">
<core:creationDate>2024-02-22</core:creationDate>
<core:externalReference>
<core:informationSystem>http://repository.gdi-de.org/schemas/adv/citygml/fdv/art.htm#_9100</core:informationSystem>
<core:externalObject>
<core:name>DEBYvAAAAAB5DOr5</core:name>
</core:externalObject>
</core:externalReference>
<gen:stringAttribute name="citygml_function">
<gen:value>31001_1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleBodenhoehe">
<gen:value>1100</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleDachhoehe">
<gen:value>5000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleLage">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Gemeindeschluessel">
<gen:value>09161000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Geometrietyp2DReferenz">
<gen:value>3000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Grundrissaktualitaet">
<gen:value>2024-01-12</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="HoeheDach">
<gen:value>378.570</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="HoeheGrund">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Methode">
<gen:value>9999</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="NiedrigsteTraufeDesGebaeudes">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<bldg:roofType>3100</bldg:roofType>
<bldg:measuredHeight uom="urn:adv:uom:m">9.668</bldg:measuredHeight>
<bldg:storeysAboveGround>3</bldg:storeysAboveGround>
<bldg:lod2Solid>
<gml:Solid srsName="EPSG:25832" srsDimension="3">
<gml:exterior>
<gml:CompositeSurface>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_f8fa960a-4fc0-4a8c-8660-70709bc89f3a_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_05cd52ff-a853-4be3-b0d2-3b74a97ab220_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_8f4c842a-3fe5-403c-873b-2d4958c6353e_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_8554e146-c14e-4656-8e9f-86ad44c1abd1_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_d2ae4a38-ccaa-4155-bbb9-04903d69e486_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_801d32a1-77f7-4173-b713-93ad069564f7_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_023ac5cb-a1bd-43a7-b6b1-ceeef4f1f3ba_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_1d0dd376-3b31-4d3f-b7f7-601bdb0a10cc_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_6bf8b6df-6ac2-4747-897f-a2a7a08221a6_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_f6bd5d9a-83f6-42f3-9884-12dfafa8afd8_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_a9ddd734-41f8-4337-83ab-a40bf736e82c_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_e46d9efb-52ae-408a-8173-40ea04c21a0b_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_c57fe8fc-f437-4062-b8c0-784fe92eec4e_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_31d8f0a3-4c9d-4c18-82aa-ef801605ebcb_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_107777354_1544d7b2-f27f-4ead-a573-a42fe86b51a8_poly"/>
</gml:CompositeSurface>
</gml:exterior>
</gml:Solid>
</bldg:lod2Solid>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_f8fa960a-4fc0-4a8c-8660-70709bc89f3a">
<gen:stringAttribute name="Flaeche">
<gen:value>14.495</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>5.885</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.787</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_f8fa960a-4fc0-4a8c-8660-70709bc89f3a_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676309.67 5403274.29 368.902 676309.67 5403274.29 374.785 676310.13 5403271.87 374.787 676310.13 5403271.87 368.902 676309.67 5403274.29 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_05cd52ff-a853-4be3-b0d2-3b74a97ab220">
<gen:stringAttribute name="Flaeche">
<gen:value>63.272</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.668</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>378.570</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_05cd52ff-a853-4be3-b0d2-3b74a97ab220_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676306.13 5403271.11 368.902 676310.13 5403271.87 368.902 676310.13 5403271.87 374.787 676306.144 5403271.113 378.57 676302.13 5403270.35 374.76 676302.13 5403270.35 368.902 676306.13 5403271.11 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_8f4c842a-3fe5-403c-873b-2d4958c6353e">
<gen:stringAttribute name="Flaeche">
<gen:value>29.474</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>5.885</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.787</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_8f4c842a-3fe5-403c-873b-2d4958c6353e_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676308.73 5403279.21 368.902 676308.73 5403279.21 374.787 676309.67 5403274.29 374.785 676309.67 5403274.29 368.902 676308.73 5403279.21 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:RoofSurface gml:id="DEBY_LOD2_107777354_8554e146-c14e-4656-8e9f-86ad44c1abd1">
<gen:stringAttribute name="Dachneigung">
<gen:value>90.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Dachorientierung">
<gen:value>-1.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Flaeche">
<gen:value>16.976</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>2.718</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>2.718</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_8554e146-c14e-4656-8e9f-86ad44c1abd1_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676299.04 5403274.51 371.62 676298.93 5403274.49 371.62 676299.81 5403269.91 371.62 676302.109 5403270.346 371.62 676300.73 5403277.576 371.62 676298.52 5403277.16 371.62 676299.04 5403274.51 371.62</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:RoofSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_d2ae4a38-ccaa-4155-bbb9-04903d69e486">
<gen:stringAttribute name="Flaeche">
<gen:value>63.272</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.668</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>378.570</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_d2ae4a38-ccaa-4155-bbb9-04903d69e486_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676308.73 5403279.21 368.902 676304.73 5403278.45 368.902 676300.73 5403277.69 368.902 676300.73 5403277.69 374.76 676304.744 5403278.453 378.57 676308.73 5403279.21 374.787 676308.73 5403279.21 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_801d32a1-77f7-4173-b713-93ad069564f7">
<gen:stringAttribute name="Flaeche">
<gen:value>0.304</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>2.718</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_801d32a1-77f7-4173-b713-93ad069564f7_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676298.93 5403274.49 368.902 676298.93 5403274.49 371.62 676299.04 5403274.51 371.62 676299.04 5403274.51 368.902 676298.93 5403274.49 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:RoofSurface gml:id="DEBY_LOD2_107777354_023ac5cb-a1bd-43a7-b6b1-ceeef4f1f3ba">
<gen:stringAttribute name="Dachneigung">
<gen:value>47.001</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Dachorientierung">
<gen:value>259.201</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Flaeche">
<gen:value>41.961</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.668</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>378.570</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>5.838</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>374.740</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_023ac5cb-a1bd-43a7-b6b1-ceeef4f1f3ba_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676306.144 5403271.113 378.57 676304.744 5403278.453 378.57 676300.73 5403277.69 374.76 676300.75 5403277.58 374.759 676300.73 5403277.576 374.74 676302.109 5403270.346 374.74 676302.13 5403270.35 374.76 676306.144 5403271.113 378.57</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:RoofSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_1d0dd376-3b31-4d3f-b7f7-601bdb0a10cc">
<gen:stringAttribute name="Flaeche">
<gen:value>22.964</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>5.838</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.740</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>2.718</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_1d0dd376-3b31-4d3f-b7f7-601bdb0a10cc_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676302.109 5403270.346 371.62 676302.109 5403270.346 374.74 676300.73 5403277.576 374.74 676300.73 5403277.576 371.62 676302.109 5403270.346 371.62</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_6bf8b6df-6ac2-4747-897f-a2a7a08221a6">
<gen:stringAttribute name="Flaeche">
<gen:value>6.486</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>5.858</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.760</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_6bf8b6df-6ac2-4747-897f-a2a7a08221a6_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676299.81 5403269.91 368.902 676302.13 5403270.35 368.902 676302.13 5403270.35 374.76 676302.109 5403270.346 374.74 676302.109 5403270.346 371.62 676299.81 5403269.91 371.62 676299.81 5403269.91 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_f6bd5d9a-83f6-42f3-9884-12dfafa8afd8">
<gen:stringAttribute name="Flaeche">
<gen:value>6.232</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>5.857</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.759</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_f6bd5d9a-83f6-42f3-9884-12dfafa8afd8_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676300.75 5403277.58 368.902 676298.52 5403277.16 368.902 676298.52 5403277.16 371.62 676300.73 5403277.576 371.62 676300.73 5403277.576 374.74 676300.75 5403277.58 374.759 676300.75 5403277.58 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_a9ddd734-41f8-4337-83ab-a40bf736e82c">
<gen:stringAttribute name="Flaeche">
<gen:value>12.677</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>2.718</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_a9ddd734-41f8-4337-83ab-a40bf736e82c_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676299.81 5403269.91 368.902 676299.81 5403269.91 371.62 676298.93 5403274.49 371.62 676298.93 5403274.49 368.902 676299.81 5403269.91 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:RoofSurface gml:id="DEBY_LOD2_107777354_e46d9efb-52ae-408a-8173-40ea04c21a0b">
<gen:stringAttribute name="Dachneigung">
<gen:value>47.001</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Dachorientierung">
<gen:value>79.201</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Flaeche">
<gen:value>41.460</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.668</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>378.570</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>5.883</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>374.785</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_e46d9efb-52ae-408a-8173-40ea04c21a0b_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676306.144 5403271.113 378.57 676310.13 5403271.87 374.787 676309.67 5403274.29 374.785 676308.73 5403279.21 374.787 676304.744 5403278.453 378.57 676306.144 5403271.113 378.57</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:RoofSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_c57fe8fc-f437-4062-b8c0-784fe92eec4e">
<gen:stringAttribute name="Flaeche">
<gen:value>7.341</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>2.718</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>371.620</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_c57fe8fc-f437-4062-b8c0-784fe92eec4e_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676299.04 5403274.51 368.902 676299.04 5403274.51 371.62 676298.52 5403277.16 371.62 676298.52 5403277.16 368.902 676299.04 5403274.51 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:GroundSurface gml:id="DEBY_LOD2_107777354_31d8f0a3-4c9d-4c18-82aa-ef801605ebcb">
<gen:stringAttribute name="Flaeche">
<gen:value>77.987</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_31d8f0a3-4c9d-4c18-82aa-ef801605ebcb_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676306.13 5403271.11 368.902 676302.13 5403270.35 368.902 676299.81 5403269.91 368.902 676298.93 5403274.49 368.902 676299.04 5403274.51 368.902 676298.52 5403277.16 368.902 676300.75 5403277.58 368.902 676300.73 5403277.69 368.902 676304.73 5403278.45 368.902 676308.73 5403279.21 368.902 676309.67 5403274.29 368.902 676310.13 5403271.87 368.902 676306.13 5403271.11 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:GroundSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_107777354_1544d7b2-f27f-4ead-a573-a42fe86b51a8">
<gen:stringAttribute name="Flaeche">
<gen:value>0.655</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>5.858</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.760</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>368.902</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_107777354_1544d7b2-f27f-4ead-a573-a42fe86b51a8_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>676300.75 5403277.58 368.902 676300.75 5403277.58 374.759 676300.73 5403277.69 374.76 676300.73 5403277.69 368.902 676300.75 5403277.58 368.902</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
</bldg:Building>
</core:cityObjectMember>
<core:cityObjectMember>
<bldg:Building gml:id="DEBY_LOD2_4392636">
<core:creationDate>2015-06-16</core:creationDate>
<core:externalReference>
<core:informationSystem>http://repository.gdi-de.org/schemas/adv/citygml/fdv/art.htm#_9100</core:informationSystem>
<core:externalObject>
<core:name>DEBYvAAAAABHpgGe</core:name>
</core:externalObject>
</core:externalReference>
<gen:stringAttribute name="citygml_function">
<gen:value>31001_1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleBodenhoehe">
<gen:value>1100</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleDachhoehe">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="DatenquelleLage">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Gemeindeschluessel">
<gen:value>09161000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Geometrietyp2DReferenz">
<gen:value>3000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Grundrissaktualitaet">
<gen:value>2024-01-12</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="HoeheDach">
<gen:value>377.067</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="HoeheGrund">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Methode">
<gen:value>1000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="NiedrigsteTraufeDesGebaeudes">
<gen:value>374.254</gen:value>
</gen:stringAttribute>
<bldg:roofType>3100</bldg:roofType>
<bldg:measuredHeight uom="urn:adv:uom:m">9.707</bldg:measuredHeight>
<bldg:lod2Solid>
<gml:Solid srsName="EPSG:25832" srsDimension="3">
<gml:exterior>
<gml:CompositeSurface>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_67b326f8-901e-42b0-81be-ffc7ef612ff8_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_9bae137a-d87f-4ae0-90ca-e88e69da6442_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_a487f445-ccf0-4f3a-b2bc-0d9611032a6b_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_755c8fe7-6d88-48b1-8df5-d42fd7e516be_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_6d743f83-7adf-426d-b9e9-64f172e5593e_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_6ea75cf9-dab9-4000-b214-da57407f9ca2_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_497abc33-2761-47f6-86fb-05ef6a589c45_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_3db6de86-598f-4ed8-8502-9c6947eb4679_poly"/>
<gml:surfaceMember xlink:href="#DEBY_LOD2_4392636_106970d8-3628-4458-8ee4-e8e951475e0e_poly"/>
</gml:CompositeSurface>
</gml:exterior>
</gml:Solid>
</bldg:lod2Solid>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_4392636_67b326f8-901e-42b0-81be-ffc7ef612ff8">
<gen:stringAttribute name="Flaeche">
<gen:value>74.056</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.707</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>377.067</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_67b326f8-901e-42b0-81be-ffc7ef612ff8_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677517.416 5403588.102 367.36 677520.404 5403590.612 367.36 677524.145 5403593.754 367.36 677524.145 5403593.754 374.254 677520.404 5403590.612 377.067 677517.416 5403588.102 374.82 677517.416 5403588.102 367.36</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_4392636_9bae137a-d87f-4ae0-90ca-e88e69da6442">
<gen:stringAttribute name="Flaeche">
<gen:value>62.062</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>6.922</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.282</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_9bae137a-d87f-4ae0-90ca-e88e69da6442_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677513.36 5403591.474 367.36 677513.36 5403591.474 374.269 677507.635 5403598.382 374.282 677507.635 5403598.382 367.36 677513.36 5403591.474 367.36</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:RoofSurface gml:id="DEBY_LOD2_4392636_a487f445-ccf0-4f3a-b2bc-0d9611032a6b">
<gen:stringAttribute name="Dachneigung">
<gen:value>60.075</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Dachorientierung">
<gen:value>52.459</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Flaeche">
<gen:value>79.187</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.707</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>377.067</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>6.894</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>374.254</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_a487f445-ccf0-4f3a-b2bc-0d9611032a6b_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677520.404 5403590.612 377.067 677524.145 5403593.754 374.254 677515.063 5403604.564 374.288 677511.353 5403601.476 377.067 677520.404 5403590.612 377.067</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:RoofSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_4392636_755c8fe7-6d88-48b1-8df5-d42fd7e516be">
<gen:stringAttribute name="Flaeche">
<gen:value>97.581</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>6.928</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.288</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_755c8fe7-6d88-48b1-8df5-d42fd7e516be_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677515.063 5403604.564 367.36 677515.063 5403604.564 374.288 677524.145 5403593.754 374.254 677524.145 5403593.754 367.36 677515.063 5403604.564 367.36</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_4392636_6d743f83-7adf-426d-b9e9-64f172e5593e">
<gen:stringAttribute name="Flaeche">
<gen:value>6.704</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>7.446</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.806</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_6d743f83-7adf-426d-b9e9-64f172e5593e_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677514.077 5403592.073 367.36 677514.077 5403592.073 374.806 677513.36 5403591.474 374.269 677513.36 5403591.474 367.36 677514.077 5403592.073 367.36</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:RoofSurface gml:id="DEBY_LOD2_4392636_6ea75cf9-dab9-4000-b214-da57407f9ca2">
<gen:stringAttribute name="Dachneigung">
<gen:value>60.074</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Dachorientierung">
<gen:value>232.459</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Flaeche">
<gen:value>73.593</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.707</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>377.067</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>6.909</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>374.269</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_6ea75cf9-dab9-4000-b214-da57407f9ca2_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677513.36 5403591.474 374.269 677514.077 5403592.073 374.806 677517.416 5403588.102 374.82 677520.404 5403590.612 377.067 677511.353 5403601.476 377.067 677507.635 5403598.382 374.282 677513.36 5403591.474 374.269</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:RoofSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:GroundSurface gml:id="DEBY_LOD2_4392636_497abc33-2761-47f6-86fb-05ef6a589c45">
<gen:stringAttribute name="Flaeche">
<gen:value>132.411</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_497abc33-2761-47f6-86fb-05ef6a589c45_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677520.404 5403590.612 367.36 677517.416 5403588.102 367.36 677514.077 5403592.073 367.36 677513.36 5403591.474 367.36 677507.635 5403598.382 367.36 677511.353 5403601.476 367.36 677515.063 5403604.564 367.36 677524.145 5403593.754 367.36 677520.404 5403590.612 367.36</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:GroundSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_4392636_3db6de86-598f-4ed8-8502-9c6947eb4679">
<gen:stringAttribute name="Flaeche">
<gen:value>80.368</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>9.707</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>377.067</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_3db6de86-598f-4ed8-8502-9c6947eb4679_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677507.635 5403598.382 374.282 677511.353 5403601.476 377.067 677515.063 5403604.564 374.288 677515.063 5403604.564 367.36 677511.353 5403601.476 367.36 677507.635 5403598.382 367.36 677507.635 5403598.382 374.282</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
<bldg:boundedBy>
<bldg:WallSurface gml:id="DEBY_LOD2_4392636_106970d8-3628-4458-8ee4-e8e951475e0e">
<gen:stringAttribute name="Flaeche">
<gen:value>38.672</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX">
<gen:value>7.460</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MAX_ASL">
<gen:value>374.820</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN">
<gen:value>0.000</gen:value>
</gen:stringAttribute>
<gen:stringAttribute name="Z_MIN_ASL">
<gen:value>367.360</gen:value>
</gen:stringAttribute>
<bldg:lod2MultiSurface>
<gml:MultiSurface srsName="EPSG:25832" srsDimension="3">
<gml:surfaceMember>
<gml:Polygon gml:id="DEBY_LOD2_4392636_106970d8-3628-4458-8ee4-e8e951475e0e_poly">
<gml:exterior>
<gml:LinearRing>
<gml:posList>677517.416 5403588.102 367.36 677517.416 5403588.102 374.82 677514.077 5403592.073 374.806 677514.077 5403592.073 367.36 677517.416 5403588.102 367.36</gml:posList>
</gml:LinearRing>
</gml:exterior>
</gml:Polygon>
</gml:surfaceMember>
</gml:MultiSurface>
</bldg:lod2MultiSurface>
</bldg:WallSurface>
</bldg:boundedBy>
</bldg:Building>
</core:cityObjectMember>
</core:CityModel>
