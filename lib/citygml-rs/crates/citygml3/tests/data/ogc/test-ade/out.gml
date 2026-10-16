<?xml version="1.0" encoding="UTF-8"?>
<core:CityModel xmlns:core="http://www.opengis.net/citygml/3.0" xmlns:app="http://www.opengis.net/citygml/appearance/3.0" xmlns:brid="http://www.opengis.net/citygml/bridge/3.0" xmlns:bldg="http://www.opengis.net/citygml/building/3.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/3.0" xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/3.0" xmlns:con="http://www.opengis.net/citygml/construction/3.0" xmlns:gen="http://www.opengis.net/citygml/generics/3.0" xmlns:luse="http://www.opengis.net/citygml/landuse/3.0" xmlns:dem="http://www.opengis.net/citygml/relief/3.0" xmlns:tran="http://www.opengis.net/citygml/transportation/3.0" xmlns:tun="http://www.opengis.net/citygml/tunnel/3.0" xmlns:veg="http://www.opengis.net/citygml/vegetation/3.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/3.0" xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:xAL="urn:oasis:names:tc:ciq:xal:3" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:test="http://www.citygml.org/ade/TestADE/2.0" xsi:schemaLocation="http://www.opengis.net/citygml/3.0 http://schemas.opengis.net/citygml/3.0/core.xsd http://www.opengis.net/citygml/appearance/3.0 http://schemas.opengis.net/citygml/appearance/3.0/appearance.xsd http://www.opengis.net/citygml/bridge/3.0 http://schemas.opengis.net/citygml/bridge/3.0/bridge.xsd http://www.opengis.net/citygml/building/3.0 http://schemas.opengis.net/citygml/building/3.0/building.xsd http://www.opengis.net/citygml/cityfurniture/3.0 http://schemas.opengis.net/citygml/cityfurniture/3.0/cityFurniture.xsd http://www.opengis.net/citygml/cityobjectgroup/3.0 http://schemas.opengis.net/citygml/cityobjectgroup/3.0/cityObjectGroup.xsd http://www.opengis.net/citygml/construction/3.0 http://schemas.opengis.net/citygml/construction/3.0/construction.xsd http://www.opengis.net/citygml/generics/3.0 http://schemas.opengis.net/citygml/generics/3.0/generics.xsd http://www.opengis.net/citygml/landuse/3.0 http://schemas.opengis.net/citygml/landuse/3.0/landUse.xsd http://www.opengis.net/citygml/relief/3.0 http://schemas.opengis.net/citygml/relief/3.0/relief.xsd http://www.opengis.net/citygml/transportation/3.0 http://schemas.opengis.net/citygml/transportation/3.0/transportation.xsd http://www.opengis.net/citygml/tunnel/3.0 http://schemas.opengis.net/citygml/tunnel/3.0/tunnel.xsd http://www.opengis.net/citygml/vegetation/3.0 http://schemas.opengis.net/citygml/vegetation/3.0/vegetation.xsd http://www.opengis.net/citygml/waterbody/3.0 http://schemas.opengis.net/citygml/waterbody/3.0/waterBody.xsd http://www.citygml.org/ade/TestADE/2.0 testADE.xsd">
 <core:cityObjectMember>
  <test:IndustrialBuilding gml:id="IndustrialBuilding_1">
   <gml:description>FZK/IAI test cases only</gml:description>
   <gml:name>Testcase-0-0_LoD2</gml:name>
   <bldg:class>1100</bldg:class>
   <bldg:roofType>1030</bldg:roofType>
   <bldg:storeysAboveGround>5</bldg:storeysAboveGround>
   <bldg:storeysBelowGround>0</bldg:storeysBelowGround>
   <bldg:storeyHeightsAboveGround uom="m">2</bldg:storeyHeightsAboveGround>
   <bldg:storeyHeightsBelowGround uom="m">0</bldg:storeyHeightsBelowGround>
   <bldg:adeOfAbstractBuilding>
    <test:BuildingProperties>
     <test:ownerName>Smith</test:ownerName>
     <test:energyPerformanceCertification>
      <test:EnergyPerformanceCertification>
       <test:certificationName>certName</test:certificationName>
       <test:certificationId>certId</test:certificationId>
      </test:EnergyPerformanceCertification>
     </test:energyPerformanceCertification>
     <test:buildingUnderground>
      <test:BuildingUnderground>
       <test:equippedWith>
        <test:LightingFacility>
         <test:electricalPower uom="W">4000.0</test:electricalPower>
        </test:LightingFacility>
       </test:equippedWith>
      </test:BuildingUnderground>
     </test:buildingUnderground>
    </test:BuildingProperties>
   </bldg:adeOfAbstractBuilding>
   <test:remark>remark</test:remark>
  </test:IndustrialBuilding>
 </core:cityObjectMember>
</core:CityModel>
