<?xml version='1.0' encoding='utf-8'?>
<!--
  Test fragment from Japan's PLATEAU 3D City Models (Kanagawa, Yokohama-shi;
  14100_yokohama-shi_city_2024_citygml_2_op), CityGML 2.0 module `bldg`,
  tile 53391458_bldg_6697_op.gml. Licence: CC BY 4.0 (Project PLATEAU, MLIT
  Japan). Source archive:
  https://assets.cms.plateau.reearth.io/assets/04/96d45a-a30b-4c9f-88c0-c1323b5a0f86/14100_yokohama-shi_city_2024_citygml_2_op.zip

  Root CityModel + gml:Envelope + the first TWO cityObjectMembers carrying
  both a `lod2Solid` and semantic surfaces, verbatim and in document order.

  Kept for what a national Japanese export carries that no European fixture
  does: `srsName` EPSG:6697 — JGD2011 + JGD2011 (vertical) height, whose axes
  are latitude (degree), longitude (degree), height (metre). It is the fixture
  for the per-axis quantisation step and for the GeoParquet (longitude,
  latitude) WKB axis order. It also carries LoD0/1/2 geometry, Roof/Wall/Ground
  semantic surfaces, a ParameterizedTexture with TexCoordList, and the `uro`
  (i-UR urban object) ADE the reader does not map.
-->
<core:CityModel xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:uro="https://www.geospatial.jp/iur/uro/3.1" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="https://www.geospatial.jp/iur/uro/3.1 ../../schemas/iur/uro/3.1/urbanObject.xsd http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/cityobjectgroup/2.0 http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd http://www.opengis.net/gml http://schemas.opengis.net/gml/3.1.1/base/gml.xsd http://www.opengis.net/citygml/appearance/2.0 http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd">
  <gml:boundedBy>
    <gml:Envelope srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3">
      <gml:lowerCorner>35.45804463285104 139.5998896941225 0</gml:lowerCorner>
      <gml:upperCorner>35.46675117051738 139.61265663694576 61.80299980164</gml:upperCorner>
    </gml:Envelope>
  </gml:boundedBy>
  <app:appearanceMember>
    <app:Appearance>
      <app:theme>rgbTexture</app:theme>
      <app:surfaceDataMember>
        <app:ParameterizedTexture>
          <app:imageURI>53391458_bldg_6697_appearance/yk141401.jpg</app:imageURI>
          <app:mimeType>image/jpg</app:mimeType>
          <app:target uri="#poly_YK141401_p3841_16">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_16">0 0.0324596 0.019786 0.0324596 0.019786 0.0572696 0.0115896 0.0572696 0 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_6">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_6">0.3688968 0.0324596 0.3766387 0.0324596 0.3766387 0.9967107 0.3688968 0.9967107 0.3688968 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_4">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_4">0.3347989 0.0324596 0.3463589 0.0324596 0.3463589 0.9967107 0.3347989 0.9967107 0.3347989 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_12">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_12">0.6960203 0.0916836 0.6960203 0.9967107 0.5348082 0.9967107 0.5348082 0.0324596 0.5960861 0.0324596 0.6629379 0.0324596 0.6629379 0.0046083 0.6818358 0.0046083 0.6818358 0.0916836 0.6960203 0.0916836</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_11">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_11">0.5282747 0.0324596 0.5348082 0.0324596 0.5348082 0.9967107 0.5282747 0.9967107 0.5282747 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_14">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_14">0.184299 0.0324596 0.184299 0.7855648 0.1507193 0.7855648 0.1507193 0.0324596 0.184299 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_1">0.9940531 0.6568419 0.9967613 0.8235146 0.9434094 0.8278273 0.9444574 0.892316 0.9433839 0.8924028 0.9428678 0.9349046 0.9435871 0.9791754 0.8458967 0.9870723 0.7563513 0.9943107 0.7560053 0.9730159 0.7438016 0.9740024 0.7444748 0.643658 0.758041 0.6425614 0.7563086 0.5359421 0.7449953 0.5368566 0.7438016 0.4633987 0.7562066 0.437788 0.9331257 0.4692971 0.9344707 0.5520682 0.9290209 0.5525087 0.9307993 0.6619551 0.9940531 0.6568419</app:textureCoordinates>
              <app:textureCoordinates ring="#line_YK141401_p3841_1_hole0">0.8452496 0.9472474 0.9068967 0.942264 0.9039106 0.7585013 0.8422635 0.7634847 0.8452496 0.9472474</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3845_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3845_0">0.9704833 0.9326722 0.9912111 0.9309967 0.9919305 0.9752676 0.9712027 0.9769431 0.9704833 0.9326722</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3844_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3844_1">0.9000944 0.3094724 0.8712569 0.3118438 0.8434441 0.314131 0.8429752 0.2857645 0.8996255 0.281106 0.9000944 0.3094724</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_9">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_9">0.4186347 0.0324596 0.5199234 0.0324596 0.5199234 0.9967107 0.4186347 0.9967107 0.4186347 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3844_4">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3844_4">0.8099173 0.1474655 0.8206502 0.1474655 0.8206502 0.1724572 0.8099173 0.1474655</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3844_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3844_0">0.8720086 0.4124144 0.8712569 0.3118438 0.9000944 0.3094724 0.8940179 0.4106045 0.8720086 0.4124144</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3843_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3843_1">0.8099173 0.1152074 0.8203156 0.1152074 0.8203156 0.1404352 0.8099173 0.1404352 0.8099173 0.1152074</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3843_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3843_0">0.9704833 0.9326722 0.9712027 0.9769431 0.9435871 0.9791754 0.9428678 0.9349046 0.9704833 0.9326722</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3842_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3842_0">0.9068967 0.942264 0.8452495 0.9472474 0.8422636 0.7634847 0.9039107 0.7585013 0.9068967 0.942264</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_2">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_2">0.2025566 0.9967107 0.2025566 0.7855648 0.1880283 0.7855648 0.1880283 0.0324596 0.2134236 0.0324596 0.2134236 0.9967107 0.2025566 0.9967107</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_15">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_15">0.1880283 0.0324596 0.1880283 0.7855648 0.184299 0.7855648 0.184299 0.0324596 0.1880283 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_3">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_3">0.2134236 0.0324596 0.3347989 0.0324596 0.3347989 0.9967107 0.2134236 0.9967107 0.2134236 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_13">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_13">0.1507193 0.0324596 0.1507193 0.7855648 0.1074334 0.7855648 0.1074334 0.0324596 0.1507193 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3846_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3846_0">0.9940531 0.6568419 0.9307993 0.6619551 0.9290209 0.5525087 0.9344707 0.5520682 0.9337012 0.5047162 0.9917178 0.5131309 0.9940531 0.6568419</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_17">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_17">0.019786 0.0324596 0.0562959 0.0324596 0.0562959 0.0572696 0.019786 0.0572696 0.019786 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3842_3">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3842_3">0.8047143 0.0960917 0.7438016 0.0960917 0.7438016 0.0046083 0.8047143 0.0046083 0.8047143 0.0960917</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_8">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_8">0.409351 0.0324596 0.4186347 0.0324596 0.4186347 0.9967107 0.409351 0.9967107 0.409351 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3846_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3846_1">0.8416917 0.4268037 0.8099173 0.4268037 0.8099173 0.2580645 0.8416917 0.2580645 0.8416917 0.4268037</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3842_2">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3842_2">0.8047143 0.4321049 0.7438016 0.4321049 0.7438016 0.30984 0.8047143 0.30984 0.8047143 0.4321049</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3844_5">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3844_5">0.8280163 0.1863359 0.8360908 0.2580645 0.8078512 0.2580645 0.8280163 0.1863359</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_5">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_5">0.3463589 0.0324596 0.3688968 0.0324596 0.3688968 0.9967107 0.3463589 0.9967107 0.3463589 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3844_3">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3844_3">0.7004132 0.9953917 0.7004132 0.0376973 0.7269741 0.0012806 0.7269741 0.9953917 0.7004132 0.9953917</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_0">0.1074334 0.0324596 0.1074334 0.7855648 0.1515259 0.7855648 0.1515259 0.9967107 0.0480995 0.9967107 0.0480995 0.0572696 0.0562959 0.0572696 0.0562959 0.0324596 0.1074334 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3842_4">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3842_4">0.8047143 0.2183565 0.7438016 0.2183565 0.7438016 0.0960917 0.8047143 0.0960917 0.8047143 0.2183565</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3845_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3845_1">0.7293388 0.9907834 0.7293388 0.1172038 0.7424498 0.1172038 0.7424498 0.9907834 0.7293388 0.9907834</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_10">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_10">0.5199234 0.0324596 0.5282747 0.0324596 0.5282747 0.9967107 0.5199234 0.9967107 0.5199234 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3843_2">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3843_2">0.7404178 0.0309391 0.7404178 0.1152074 0.7272727 0.1152074 0.7272727 0.0309391 0.7404178 0.0309391</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3841_7">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3841_7">0.3766387 0.0324596 0.409351 0.0324596 0.409351 0.9967107 0.3766387 0.9967107 0.3766387 0.0324596</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3842_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3842_1">0.8047143 0.30984 0.7438016 0.30984 0.7438016 0.2183565 0.8047143 0.2183565 0.8047143 0.30984</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141401_p3844_2">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141401_p3844_2">0.872298 0.4299222 0.8429752 0.4323335 0.8432232 0.369819 0.844363 0.3697253 0.8434441 0.314131 0.8712569 0.3118438 0.872298 0.4299222</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
        </app:ParameterizedTexture>
      </app:surfaceDataMember>
      <app:surfaceDataMember>
        <app:ParameterizedTexture>
          <app:imageURI>53391458_bldg_6697_appearance/yk141429.jpg</app:imageURI>
          <app:mimeType>image/jpg</app:mimeType>
          <app:target uri="#poly_YK141429_p3840_4">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141429_p3840_4">0.3250427 0.0278393 0.5639279 0.0278393 0.5639279 1 0.3250427 1 0.3250427 0.0278393</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141429_p3840_1">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141429_p3840_1">0.5639279 0.0278393 0.6500854 0.0278393 0.6500854 1 0.5639279 1 0.5639279 0.0278393</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141429_p3840_2">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141429_p3840_2">0 0.0278393 0.2388851 0.0278393 0.2388851 1 0 1 0 0.0278393</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141429_p3840_0">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141429_p3840_0">0.9980023 0.1456162 0.9980023 0.9857143 0.6547368 0.9857143 0.6547368 0.1456162 0.9980023 0.1456162</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
          <app:target uri="#poly_YK141429_p3840_3">
            <app:TexCoordList>
              <app:textureCoordinates ring="#line_YK141429_p3840_3">0.2388851 0.0278393 0.3250427 0.0278393 0.3250427 1 0.2388851 1 0.2388851 0.0278393</app:textureCoordinates>
            </app:TexCoordList>
          </app:target>
        </app:ParameterizedTexture>
      </app:surfaceDataMember>
    </app:Appearance>
  </app:appearanceMember>
  <core:cityObjectMember>
    <bldg:Building gml:id="bldg_05e35e6d-e88e-49a1-bb37-a921263899ea">
      <core:creationDate>2024-03-22</core:creationDate>
      <bldg:measuredHeight uom="m">34.5</bldg:measuredHeight>
      <bldg:lod0FootPrint>
        <gml:MultiSurface>
          <gml:surfaceMember>
            <gml:Polygon>
              <gml:exterior>
                <gml:LinearRing>
                  <gml:posList>35.465501054428856 139.6123486302035 0 35.46549384511128 139.61234966689923 0 35.46544911230046 139.61242509938833 0 35.46545413712893 139.61243074346643 0 35.4654391870399 139.61245518733176 0 35.46548701184845 139.61250890117742 0 35.46550050602235 139.61251885775698 0 35.46550678637487 139.61252591204203 0 35.46557528214442 139.61243479303192 0 35.46557226074537 139.61243139998663 0 35.465575809039265 139.61242668001955 0 35.46552725773827 139.61237645319383 0 35.46552331289411 139.61238170091264 0 35.46550818691663 139.61236471152156 0 35.465511476234845 139.61236033603012 0 35.465501054428856 139.6123486302035 0</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </bldg:lod0FootPrint>
      <bldg:lod1Solid>
        <gml:Solid>
          <gml:exterior>
            <gml:CompositeSurface>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465501054428856 139.6123486302035 2.094 35.465511476234845 139.61236033603012 2.094 35.46550818691663 139.61236471152156 2.094 35.46552331289411 139.61238170091264 2.094 35.46552725773827 139.61237645319383 2.094 35.465575809039265 139.61242668001955 2.094 35.46557226074537 139.61243139998663 2.094 35.46557528214442 139.61243479303192 2.094 35.46550678637487 139.61252591204203 2.094 35.46550050602235 139.61251885775698 2.094 35.46548701184845 139.61250890117742 2.094 35.4654391870399 139.61245518733176 2.094 35.46545413712893 139.61243074346643 2.094 35.46544911230046 139.61242509938833 2.094 35.46549384511128 139.61234966689923 2.094 35.465501054428856 139.6123486302035 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465501054428856 139.6123486302035 2.094 35.46549384511128 139.61234966689923 2.094 35.46549384511128 139.61234966689923 32.495 35.465501054428856 139.6123486302035 32.495 35.465501054428856 139.6123486302035 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46549384511128 139.61234966689923 2.094 35.46544911230046 139.61242509938833 2.094 35.46544911230046 139.61242509938833 32.495 35.46549384511128 139.61234966689923 32.495 35.46549384511128 139.61234966689923 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46544911230046 139.61242509938833 2.094 35.46545413712893 139.61243074346643 2.094 35.46545413712893 139.61243074346643 32.495 35.46544911230046 139.61242509938833 32.495 35.46544911230046 139.61242509938833 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46545413712893 139.61243074346643 2.094 35.4654391870399 139.61245518733176 2.094 35.4654391870399 139.61245518733176 32.495 35.46545413712893 139.61243074346643 32.495 35.46545413712893 139.61243074346643 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.4654391870399 139.61245518733176 2.094 35.46548701184845 139.61250890117742 2.094 35.46548701184845 139.61250890117742 32.495 35.4654391870399 139.61245518733176 32.495 35.4654391870399 139.61245518733176 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46548701184845 139.61250890117742 2.094 35.46550050602235 139.61251885775698 2.094 35.46550050602235 139.61251885775698 32.495 35.46548701184845 139.61250890117742 32.495 35.46548701184845 139.61250890117742 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46550050602235 139.61251885775698 2.094 35.46550678637487 139.61252591204203 2.094 35.46550678637487 139.61252591204203 32.495 35.46550050602235 139.61251885775698 32.495 35.46550050602235 139.61251885775698 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46550678637487 139.61252591204203 2.094 35.46557528214442 139.61243479303192 2.094 35.46557528214442 139.61243479303192 32.495 35.46550678637487 139.61252591204203 32.495 35.46550678637487 139.61252591204203 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46557528214442 139.61243479303192 2.094 35.46557226074537 139.61243139998663 2.094 35.46557226074537 139.61243139998663 32.495 35.46557528214442 139.61243479303192 32.495 35.46557528214442 139.61243479303192 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46557226074537 139.61243139998663 2.094 35.465575809039265 139.61242668001955 2.094 35.465575809039265 139.61242668001955 32.495 35.46557226074537 139.61243139998663 32.495 35.46557226074537 139.61243139998663 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465575809039265 139.61242668001955 2.094 35.46552725773827 139.61237645319383 2.094 35.46552725773827 139.61237645319383 32.495 35.465575809039265 139.61242668001955 32.495 35.465575809039265 139.61242668001955 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46552725773827 139.61237645319383 2.094 35.46552331289411 139.61238170091264 2.094 35.46552331289411 139.61238170091264 32.495 35.46552725773827 139.61237645319383 32.495 35.46552725773827 139.61237645319383 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46552331289411 139.61238170091264 2.094 35.46550818691663 139.61236471152156 2.094 35.46550818691663 139.61236471152156 32.495 35.46552331289411 139.61238170091264 32.495 35.46552331289411 139.61238170091264 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.46550818691663 139.61236471152156 2.094 35.465511476234845 139.61236033603012 2.094 35.465511476234845 139.61236033603012 32.495 35.46550818691663 139.61236471152156 32.495 35.46550818691663 139.61236471152156 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465511476234845 139.61236033603012 2.094 35.465501054428856 139.6123486302035 2.094 35.465501054428856 139.6123486302035 32.495 35.465511476234845 139.61236033603012 32.495 35.465511476234845 139.61236033603012 2.094</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465501054428856 139.6123486302035 32.495 35.46549384511128 139.61234966689923 32.495 35.46544911230046 139.61242509938833 32.495 35.46545413712893 139.61243074346643 32.495 35.4654391870399 139.61245518733176 32.495 35.46548701184845 139.61250890117742 32.495 35.46550050602235 139.61251885775698 32.495 35.46550678637487 139.61252591204203 32.495 35.46557528214442 139.61243479303192 32.495 35.46557226074537 139.61243139998663 32.495 35.465575809039265 139.61242668001955 32.495 35.46552725773827 139.61237645319383 32.495 35.46552331289411 139.61238170091264 32.495 35.46550818691663 139.61236471152156 32.495 35.465511476234845 139.61236033603012 32.495 35.465501054428856 139.6123486302035 32.495</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:CompositeSurface>
          </gml:exterior>
        </gml:Solid>
      </bldg:lod1Solid>
      <bldg:lod2Solid>
        <gml:Solid>
          <gml:exterior>
            <gml:CompositeSurface>
              <gml:surfaceMember xlink:href="#poly_YK141401_p3846_0" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3846_1" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_0" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_1" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_2" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_3" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_4" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_5" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_6" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_7" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_8" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_9" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_10" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_11" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_12" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_13" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_14" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_15" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_16" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3841_17" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3845_0" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3845_1" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3844_0" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3844_1" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3844_2" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3844_3" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3844_4" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3844_5" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3843_0" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3843_1" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3843_2" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3842_0" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3842_1" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3842_2" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3842_3" />
              <gml:surfaceMember xlink:href="#poly_YK141401_p3842_4" />
              <gml:surfaceMember xlink:href="#poly_YK141401_b_0" />
            </gml:CompositeSurface>
          </gml:exterior>
        </gml:Solid>
      </bldg:lod2Solid>
      <bldg:boundedBy>
        <bldg:GroundSurface gml:id="gnd_YK141401_b_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_b_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_b_0">
                      <gml:posList>35.46550678652565 139.61252591232744 2.29487853 35.46550050572297 139.6125188580844 2.29487728 35.465487011888 139.61250890087234 2.29487575 35.46543918707235 139.61245518683742 2.2948728 35.46545413712579 139.61243074361224 2.29487047 35.465449112144924 139.6124250998547 2.29487067 35.465493845288584 139.61234966729089 2.29487115 35.465501054800995 139.61234863066284 2.29487137 35.46551147642894 139.61236033560164 2.29487095 35.46550818703145 139.61236471145966 2.29487056 35.46552331334257 139.61238170042122 2.29487065 35.465527257780415 139.61237645316726 2.29487112 35.46557580937983 139.6124266797814 2.2948768 35.46557226111085 139.61243140001338 2.29487634 35.465575282251784 139.61243479318344 2.29487696 35.46550678652565 139.61252591232744 2.29487853</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:GroundSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3846_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3846_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3846_0">
                      <gml:posList>35.46545957566981 139.61247808599964 8.89050809 35.46547796695968 139.6124536202751 8.89050555 35.46546243960184 139.61243618086664 8.89050522 35.46546085507376 139.61243828874666 8.89050538 35.465454137160535 139.61243074358745 8.8905055 35.465439187122556 139.61245518678737 8.89050783 35.46545957566981 139.61247808599964 8.89050809</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3845_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3845_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3845_0">
                      <gml:posList>35.46550653234217 139.61251084038665 30.56551138 35.46550050566573 139.61251885758796 30.56551232 35.465506786440486 139.61252591179965 30.56551356 35.465512813111445 139.61251789459246 30.56551263 35.46550653234217 139.61251084038665 30.56551138</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3841_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_1">
                      <gml:posList>35.46545957577362 139.6124780857368 32.41550809 35.46548322184609 139.61250464363036 32.41551025 35.46549873406959 139.61248400790103 32.41550802 35.46550788317005 139.61249428364232 32.41550939 35.4655081952978 139.61249386842323 32.41550935 35.465514561642614 139.6125001590827 32.41551038 35.46552084240391 139.61250721327727 32.41551163 35.46554924622768 139.6124694279774 32.41551026 35.46557528183645 139.61243479305105 32.41551199 35.46557226070977 139.61243139989693 32.41551137 35.465575808962036 139.61242667968736 32.41551183 35.46552725759265 139.61237645331013 32.41550616 35.465523313173506 139.61238170053926 32.41550568 35.46550818693401 139.61236471165782 32.41550559 35.465511476315946 139.61236033582054 32.41550598 35.46550105473741 139.61234863093694 32.4155064 35.46549384525914 139.61234966756004 32.41550618 35.46544911232748 139.61242509976805 32.4155057 35.46546085517279 139.6124382886304 32.41550538 35.465462439695024 139.61243618075818 32.41550522 35.46547796699542 139.61245362010231 32.41550555 35.46545957577362 139.6124780857368 32.41550809</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                  <gml:interior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_1_hole0">
                      <gml:posList>35.46554359621813 139.6124630822054 32.41550913 35.46552567209915 139.6124869264381 32.4155096 35.46549960145231 139.61245764536721 32.41550586 35.465517525565595 139.61243380113547 32.41550538 35.46554359621813 139.6124630822054 32.41550913</gml:posList>
                    </gml:LinearRing>
                  </gml:interior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3844_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3844_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3844_1">
                      <gml:posList>35.46548701189084 139.61250890040284 31.64051078 35.46549490830293 139.612498395916 31.64050952 35.465502524130045 139.61248826468557 31.64050855 35.46549873407088 139.61248400791038 31.64050802 35.465483221845574 139.61250464364227 31.64051025 35.46548701189084 139.61250890040284 31.64051078</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3842_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3842_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3842_0">
                      <gml:posList>35.46552567207309 139.6124869263833 36.78050959 35.46554359617967 139.61246308216684 36.78050913 35.4655175255451 139.61243380111702 36.78050538 35.46549960144414 139.61245764533228 36.78050586 35.46552567207309 139.6124869263833 36.78050959</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3844_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3844_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3844_0">
                      <gml:posList>35.46550653234217 139.61251084038665 30.56551138 35.46549490830293 139.612498395916 31.64050952 35.46548701189084 139.61250890040284 31.64051078 35.46550050566573 139.61251885758796 30.56551232 35.46550653234217 139.61251084038665 30.56551138</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3844_2">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3844_2">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3844_2">
                      <gml:posList>35.465506532334075 139.61251084034225 33.28551138 35.46551456163898 139.61250015907007 33.28551038 35.4655081952978 139.61249386842323 32.41550935 35.46550788317005 139.61249428364232 32.41550939 35.465502524130045 139.61248826468557 31.64050855 35.46549490830293 139.612498395916 31.64050952 35.465506532334075 139.61251084034225 33.28551138</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141401_p3843_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3843_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3843_0">
                      <gml:posList>35.465506532334075 139.61251084034225 33.28551138 35.465512813100624 139.6125178945451 33.28551263 35.46552084239929 139.6125072132635 33.28551163 35.46551456163898 139.61250015907007 33.28551038 35.465506532334075 139.61251084034225 33.28551138</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_12">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_12">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_12">
                      <gml:posList>35.465506786440486 139.61252591179965 30.56551356 35.46550678652565 139.61252591232744 2.29487853 35.465575282251784 139.61243479318344 2.29487696 35.46557528183645 139.61243479305105 32.41551199 35.46554924622768 139.6124694279774 32.41551026 35.46552084240391 139.61250721327727 32.41551163 35.46552084239929 139.6125072132635 33.28551163 35.465512813100624 139.6125178945451 33.28551263 35.465512813111445 139.61251789459246 30.56551263 35.465506786440486 139.61252591179965 30.56551356</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_0">
                      <gml:posList>35.46545957577362 139.6124780857368 32.41550809 35.46545957566981 139.61247808599964 8.89050809 35.465439187122556 139.61245518678737 8.89050783 35.46543918707235 139.61245518683742 2.2948728 35.465487011888 139.61250890087234 2.29487575 35.46548701189084 139.61250890040284 31.64051078 35.465483221845574 139.61250464364227 31.64051025 35.46548322184609 139.61250464363036 32.41551025 35.46545957577362 139.6124780857368 32.41550809</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3843_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3843_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3843_1">
                      <gml:posList>35.46552084239929 139.6125072132635 33.28551163 35.46552084240391 139.61250721327727 32.41551163 35.465514561642614 139.6125001590827 32.41551038 35.46551456163898 139.61250015907007 33.28551038 35.46552084239929 139.6125072132635 33.28551163</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_11">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_11">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_11">
                      <gml:posList>35.46557226070977 139.61243139989693 32.41551137 35.46557528183645 139.61243479305105 32.41551199 35.465575282251784 139.61243479318344 2.29487696 35.46557226111085 139.61243140001338 2.29487634 35.46557226070977 139.61243139989693 32.41551137</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_10">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_10">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_10">
                      <gml:posList>35.465575808962036 139.61242667968736 32.41551183 35.46557226070977 139.61243139989693 32.41551137 35.46557226111085 139.61243140001338 2.29487634 35.46557580937983 139.6124266797814 2.2948768 35.465575808962036 139.61242667968736 32.41551183</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_7">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_7">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_7">
                      <gml:posList>35.46550818693401 139.61236471165782 32.41550559 35.465523313173506 139.61238170053926 32.41550568 35.46552331334257 139.61238170042122 2.29487065 35.46550818703145 139.61236471145966 2.29487056 35.46550818693401 139.61236471165782 32.41550559</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_13">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_13">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_13">
                      <gml:posList>35.46547796699542 139.61245362010231 32.41550555 35.46547796695968 139.6124536202751 8.89050555 35.46545957566981 139.61247808599964 8.89050809 35.46545957577362 139.6124780857368 32.41550809 35.46547796699542 139.61245362010231 32.41550555</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_8">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_8">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_8">
                      <gml:posList>35.465523313173506 139.61238170053926 32.41550568 35.46552725759265 139.61237645331013 32.41550616 35.465527257780415 139.61237645316726 2.29487112 35.46552331334257 139.61238170042122 2.29487065 35.465523313173506 139.61238170053926 32.41550568</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3842_4">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3842_4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3842_4">
                      <gml:posList>35.46549960144414 139.61245764533228 36.78050586 35.46549960145231 139.61245764536721 32.41550586 35.46552567209915 139.6124869264381 32.4155096 35.46552567207309 139.6124869263833 36.78050959 35.46549960144414 139.61245764533228 36.78050586</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_16">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_16">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_16">
                      <gml:posList>35.46550788317005 139.61249428364232 32.41550939 35.46549873406959 139.61248400790103 32.41550802 35.46549873407088 139.61248400791038 31.64050802 35.465502524130045 139.61248826468557 31.64050855 35.46550788317005 139.61249428364232 32.41550939</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_9">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_9">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_9">
                      <gml:posList>35.46552725759265 139.61237645331013 32.41550616 35.465575808962036 139.61242667968736 32.41551183 35.46557580937983 139.6124266797814 2.2948768 35.465527257780415 139.61237645316726 2.29487112 35.46552725759265 139.61237645331013 32.41550616</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3845_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3845_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3845_1">
                      <gml:posList>35.46550678652565 139.61252591232744 2.29487853 35.465506786440486 139.61252591179965 30.56551356 35.46550050566573 139.61251885758796 30.56551232 35.46550050572297 139.6125188580844 2.29487728 35.46550678652565 139.61252591232744 2.29487853</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3842_2">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3842_2">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3842_2">
                      <gml:posList>35.46554359617967 139.61246308216684 36.78050913 35.46554359621813 139.6124630822054 32.41550913 35.465517525565595 139.61243380113547 32.41550538 35.4655175255451 139.61243380111702 36.78050538 35.46554359617967 139.61246308216684 36.78050913</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3843_2">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3843_2">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3843_2">
                      <gml:posList>35.465506532334075 139.61251084034225 33.28551138 35.46550653234217 139.61251084038665 30.56551138 35.465512813111445 139.61251789459246 30.56551263 35.465512813100624 139.6125178945451 33.28551263 35.465506532334075 139.61251084034225 33.28551138</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_17">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_17">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_17">
                      <gml:posList>35.46549873406959 139.61248400790103 32.41550802 35.46548322184609 139.61250464363036 32.41551025 35.465483221845574 139.61250464364227 31.64051025 35.46549873407088 139.61248400791038 31.64050802 35.46549873406959 139.61248400790103 32.41550802</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3842_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3842_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3842_1">
                      <gml:posList>35.4655175255451 139.61243380111702 36.78050538 35.465517525565595 139.61243380113547 32.41550538 35.46549960145231 139.61245764536721 32.41550586 35.46549960144414 139.61245764533228 36.78050586 35.4655175255451 139.61243380111702 36.78050538</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3842_3">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3842_3">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3842_3">
                      <gml:posList>35.46552567207309 139.6124869263833 36.78050959 35.46552567209915 139.6124869264381 32.4155096 35.46554359621813 139.6124630822054 32.41550913 35.46554359617967 139.61246308216684 36.78050913 35.46552567207309 139.6124869263833 36.78050959</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3846_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3846_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3846_1">
                      <gml:posList>35.465439187122556 139.61245518678737 8.89050783 35.465454137160535 139.61243074358745 8.8905055 35.46545413712579 139.61243074361224 2.29487047 35.46543918707235 139.61245518683742 2.2948728 35.465439187122556 139.61245518678737 8.89050783</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_3">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_3">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_3">
                      <gml:posList>35.46544911232748 139.61242509976805 32.4155057 35.46549384525914 139.61234966756004 32.41550618 35.465493845288584 139.61234966729089 2.29487115 35.465449112144924 139.6124250998547 2.29487067 35.46544911232748 139.61242509976805 32.4155057</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_15">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_15">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_15">
                      <gml:posList>35.46546085517279 139.6124382886304 32.41550538 35.46546085507376 139.61243828874666 8.89050538 35.46546243960184 139.61243618086664 8.89050522 35.465462439695024 139.61243618075818 32.41550522 35.46546085517279 139.6124382886304 32.41550538</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3844_5">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3844_5">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3844_5">
                      <gml:posList>35.465506532334075 139.61251084034225 33.28551138 35.46549490830293 139.612498395916 31.64050952 35.46550653234217 139.61251084038665 30.56551138 35.465506532334075 139.61251084034225 33.28551138</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_2">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_2">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_2">
                      <gml:posList>35.46545413712579 139.61243074361224 2.29487047 35.465454137160535 139.61243074358745 8.8905055 35.46546085507376 139.61243828874666 8.89050538 35.46546085517279 139.6124382886304 32.41550538 35.46544911232748 139.61242509976805 32.4155057 35.465449112144924 139.6124250998547 2.29487067 35.46545413712579 139.61243074361224 2.29487047</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_4">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_4">
                      <gml:posList>35.46549384525914 139.61234966756004 32.41550618 35.46550105473741 139.61234863093694 32.4155064 35.465501054800995 139.61234863066284 2.29487137 35.465493845288584 139.61234966729089 2.29487115 35.46549384525914 139.61234966756004 32.41550618</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_5">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_5">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_5">
                      <gml:posList>35.46550105473741 139.61234863093694 32.4155064 35.465511476315946 139.61236033582054 32.41550598 35.46551147642894 139.61236033560164 2.29487095 35.465501054800995 139.61234863066284 2.29487137 35.46550105473741 139.61234863093694 32.4155064</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_6">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_6">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_6">
                      <gml:posList>35.465511476315946 139.61236033582054 32.41550598 35.46550818693401 139.61236471165782 32.41550559 35.46550818703145 139.61236471145966 2.29487056 35.46551147642894 139.61236033560164 2.29487095 35.465511476315946 139.61236033582054 32.41550598</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3844_3">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3844_3">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3844_3">
                      <gml:posList>35.46550050572297 139.6125188580844 2.29487728 35.46550050566573 139.61251885758796 30.56551232 35.46548701189084 139.61250890040284 31.64051078 35.465487011888 139.61250890087234 2.29487575 35.46550050572297 139.6125188580844 2.29487728</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3844_4">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3844_4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3844_4">
                      <gml:posList>35.46551456163898 139.61250015907007 33.28551038 35.465514561642614 139.6125001590827 32.41551038 35.4655081952978 139.61249386842323 32.41550935 35.46551456163898 139.61250015907007 33.28551038</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141401_p3841_14">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141401_p3841_14">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141401_p3841_14">
                      <gml:posList>35.465462439695024 139.61243618075818 32.41550522 35.46546243960184 139.61243618086664 8.89050522 35.46547796695968 139.6124536202751 8.89050555 35.46547796699542 139.61245362010231 32.41550555 35.465462439695024 139.61243618075818 32.41550522</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <uro:buildingDetailAttribute>
        <uro:BuildingDetailAttribute>
          <uro:surveyYear>2020</uro:surveyYear>
        </uro:BuildingDetailAttribute>
      </uro:buildingDetailAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:RiverFloodingRiskAttribute>
          <uro:description codeSpace="../../codelists/RiverFloodingRiskAttribute_description.xml">15</uro:description>
          <uro:rank codeSpace="../../codelists/RiverFloodingRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.244</uro:depth>
          <uro:adminType codeSpace="../../codelists/RiverFloodingRiskAttribute_adminType.xml">2</uro:adminType>
          <uro:scale codeSpace="../../codelists/RiverFloodingRiskAttribute_scale.xml">1</uro:scale>
        </uro:RiverFloodingRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:RiverFloodingRiskAttribute>
          <uro:description codeSpace="../../codelists/RiverFloodingRiskAttribute_description.xml">15</uro:description>
          <uro:rank codeSpace="../../codelists/RiverFloodingRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.452</uro:depth>
          <uro:adminType codeSpace="../../codelists/RiverFloodingRiskAttribute_adminType.xml">2</uro:adminType>
          <uro:scale codeSpace="../../codelists/RiverFloodingRiskAttribute_scale.xml">2</uro:scale>
          <uro:duration uom="hour">0.3167</uro:duration>
        </uro:RiverFloodingRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:TsunamiRiskAttribute>
          <uro:description codeSpace="../../codelists/TsunamiRiskAttribute_description.xml">1</uro:description>
          <uro:rank codeSpace="../../codelists/TsunamiRiskAttribute_rank.xml">2</uro:rank>
          <uro:depth uom="m">0.500</uro:depth>
        </uro:TsunamiRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:HighTideRiskAttribute>
          <uro:description codeSpace="../../codelists/HighTideRiskAttribute_description.xml">1</uro:description>
          <uro:rank codeSpace="../../codelists/HighTideRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.222</uro:depth>
        </uro:HighTideRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:buildingIDAttribute>
        <uro:BuildingIDAttribute>
          <uro:buildingID>14100-bldg-119358</uro:buildingID>
          <uro:prefecture codeSpace="../../codelists/Common_localPublicAuthorities.xml">14</uro:prefecture>
          <uro:city codeSpace="../../codelists/Common_localPublicAuthorities.xml">14103</uro:city>
        </uro:BuildingIDAttribute>
      </uro:buildingIDAttribute>
      <uro:bldgDataQualityAttribute>
        <uro:DataQualityAttribute>
          <uro:geometrySrcDescLod0 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod0>
          <uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod1>
          <uro:geometrySrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod2>
          <uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">999</uro:geometrySrcDescLod3>
          <uro:geometrySrcDescLod4 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">999</uro:geometrySrcDescLod4>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">201</uro:thematicSrcDesc>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">400</uro:thematicSrcDesc>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">023</uro:thematicSrcDesc>
          <uro:appearanceSrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">1</uro:appearanceSrcDescLod2>
          <uro:appearanceSrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">99</uro:appearanceSrcDescLod3>
          <uro:appearanceSrcDescLod4 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">99</uro:appearanceSrcDescLod4>
          <uro:lodType codeSpace="../../codelists/Building_lodType.xml">2.0</uro:lodType>
          <uro:lod1HeightType codeSpace="../../codelists/DataQualityAttribute_lod1HeightType.xml">2</uro:lod1HeightType>
          <uro:publicSurveyDataQualityAttribute>
            <uro:PublicSurveyDataQualityAttribute>
              <uro:srcScaleLod0 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod0>
              <uro:srcScaleLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod1>
              <uro:srcScaleLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod2>
              <uro:publicSurveySrcDescLod0 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod0>
              <uro:publicSurveySrcDescLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod1>
              <uro:publicSurveySrcDescLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod2>
            </uro:PublicSurveyDataQualityAttribute>
          </uro:publicSurveyDataQualityAttribute>
        </uro:DataQualityAttribute>
      </uro:bldgDataQualityAttribute>
    </bldg:Building>
  </core:cityObjectMember>
<core:cityObjectMember>
    <bldg:Building gml:id="bldg_c3586651-005f-4a93-9f37-d35734177467">
      <core:creationDate>2024-03-22</core:creationDate>
      <bldg:measuredHeight uom="m">9.7</bldg:measuredHeight>
      <bldg:lod0FootPrint>
        <gml:MultiSurface>
          <gml:surfaceMember>
            <gml:Polygon>
              <gml:exterior>
                <gml:LinearRing>
                  <gml:posList>35.4653544573463 139.6124036282542 0 35.4653241892489 139.61245637327627 0 35.465443832513465 139.6125589567429 0 35.465474100654966 139.61250621168065 0 35.4653544573463 139.6124036282542 0</gml:posList>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </bldg:lod0FootPrint>
      <bldg:lod1Solid>
        <gml:Solid>
          <gml:exterior>
            <gml:CompositeSurface>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.4653544573463 139.6124036282542 1.825 35.465474100654966 139.61250621168065 1.825 35.465443832513465 139.6125589567429 1.825 35.4653241892489 139.61245637327627 1.825 35.4653544573463 139.6124036282542 1.825</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.4653544573463 139.6124036282542 1.825 35.4653241892489 139.61245637327627 1.825 35.4653241892489 139.61245637327627 11.834 35.4653544573463 139.6124036282542 11.834 35.4653544573463 139.6124036282542 1.825</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.4653241892489 139.61245637327627 1.825 35.465443832513465 139.6125589567429 1.825 35.465443832513465 139.6125589567429 11.834 35.4653241892489 139.61245637327627 11.834 35.4653241892489 139.61245637327627 1.825</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465443832513465 139.6125589567429 1.825 35.465474100654966 139.61250621168065 1.825 35.465474100654966 139.61250621168065 11.834 35.465443832513465 139.6125589567429 11.834 35.465443832513465 139.6125589567429 1.825</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.465474100654966 139.61250621168065 1.825 35.4653544573463 139.6124036282542 1.825 35.4653544573463 139.6124036282542 11.834 35.465474100654966 139.61250621168065 11.834 35.465474100654966 139.61250621168065 1.825</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon>
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:posList>35.4653544573463 139.6124036282542 11.834 35.4653241892489 139.61245637327627 11.834 35.465443832513465 139.6125589567429 11.834 35.465474100654966 139.61250621168065 11.834 35.4653544573463 139.6124036282542 11.834</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:CompositeSurface>
          </gml:exterior>
        </gml:Solid>
      </bldg:lod1Solid>
      <bldg:lod2Solid>
        <gml:Solid>
          <gml:exterior>
            <gml:CompositeSurface>
              <gml:surfaceMember xlink:href="#poly_YK141429_p3840_0" />
              <gml:surfaceMember xlink:href="#poly_YK141429_p3840_1" />
              <gml:surfaceMember xlink:href="#poly_YK141429_p3840_2" />
              <gml:surfaceMember xlink:href="#poly_YK141429_p3840_3" />
              <gml:surfaceMember xlink:href="#poly_YK141429_p3840_4" />
              <gml:surfaceMember xlink:href="#poly_YK141429_b_0" />
            </gml:CompositeSurface>
          </gml:exterior>
        </gml:Solid>
      </bldg:lod2Solid>
      <bldg:boundedBy>
        <bldg:GroundSurface gml:id="gnd_YK141429_b_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141429_b_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141429_b_0">
                      <gml:posList>35.46535445752998 139.61240362798782 1.90207135 35.46547410093242 139.61250621207765 1.90207825 35.4654438321562 139.6125589572584 1.90207809 35.46532418879793 139.61245637312888 1.90207123 35.46535445752998 139.61240362798782 1.90207135</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:GroundSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="roof_YK141429_p3840_0">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141429_p3840_0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141429_p3840_0">
                      <gml:posList>35.465443832056145 139.612558957116 11.62634045 35.4654741007861 139.61250621201557 11.6263406 35.46535445756666 139.61240362808206 11.62633371 35.46532418888098 139.61245637314275 11.62633359 35.465443832056145 139.612558957116 11.62634045</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141429_p3840_2">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141429_p3840_2">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141429_p3840_2">
                      <gml:posList>35.46535445756666 139.61240362808206 11.62633371 35.4654741007861 139.61250621201557 11.6263406 35.46547410093242 139.61250621207765 1.90207825 35.46535445752998 139.61240362798782 1.90207135 35.46535445756666 139.61240362808206 11.62633371</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141429_p3840_1">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141429_p3840_1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141429_p3840_1">
                      <gml:posList>35.46532418888098 139.61245637314275 11.62633359 35.46535445756666 139.61240362808206 11.62633371 35.46535445752998 139.61240362798782 1.90207135 35.46532418879793 139.61245637312888 1.90207123 35.46532418888098 139.61245637314275 11.62633359</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141429_p3840_4">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141429_p3840_4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141429_p3840_4">
                      <gml:posList>35.465443832056145 139.612558957116 11.62634045 35.46532418888098 139.61245637314275 11.62633359 35.46532418879793 139.61245637312888 1.90207123 35.4654438321562 139.6125589572584 1.90207809 35.465443832056145 139.612558957116 11.62634045</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="wall_YK141429_p3840_3">
          <bldg:lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="poly_YK141429_p3840_3">
                  <gml:exterior>
                    <gml:LinearRing gml:id="line_YK141429_p3840_3">
                      <gml:posList>35.4654741007861 139.61250621201557 11.6263406 35.465443832056145 139.612558957116 11.62634045 35.4654438321562 139.6125589572584 1.90207809 35.46547410093242 139.61250621207765 1.90207825 35.4654741007861 139.61250621201557 11.6263406</gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <uro:buildingDetailAttribute>
        <uro:BuildingDetailAttribute>
          <uro:surveyYear>2020</uro:surveyYear>
        </uro:BuildingDetailAttribute>
      </uro:buildingDetailAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:RiverFloodingRiskAttribute>
          <uro:description codeSpace="../../codelists/RiverFloodingRiskAttribute_description.xml">15</uro:description>
          <uro:rank codeSpace="../../codelists/RiverFloodingRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.064</uro:depth>
          <uro:adminType codeSpace="../../codelists/RiverFloodingRiskAttribute_adminType.xml">2</uro:adminType>
          <uro:scale codeSpace="../../codelists/RiverFloodingRiskAttribute_scale.xml">1</uro:scale>
        </uro:RiverFloodingRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:RiverFloodingRiskAttribute>
          <uro:description codeSpace="../../codelists/RiverFloodingRiskAttribute_description.xml">15</uro:description>
          <uro:rank codeSpace="../../codelists/RiverFloodingRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.110</uro:depth>
          <uro:adminType codeSpace="../../codelists/RiverFloodingRiskAttribute_adminType.xml">2</uro:adminType>
          <uro:scale codeSpace="../../codelists/RiverFloodingRiskAttribute_scale.xml">2</uro:scale>
          <uro:duration uom="hour">0.2167</uro:duration>
        </uro:RiverFloodingRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:TsunamiRiskAttribute>
          <uro:description codeSpace="../../codelists/TsunamiRiskAttribute_description.xml">1</uro:description>
          <uro:rank codeSpace="../../codelists/TsunamiRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.300</uro:depth>
        </uro:TsunamiRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:bldgDisasterRiskAttribute>
        <uro:HighTideRiskAttribute>
          <uro:description codeSpace="../../codelists/HighTideRiskAttribute_description.xml">1</uro:description>
          <uro:rank codeSpace="../../codelists/HighTideRiskAttribute_rank.xml">1</uro:rank>
          <uro:depth uom="m">0.063</uro:depth>
        </uro:HighTideRiskAttribute>
      </uro:bldgDisasterRiskAttribute>
      <uro:buildingIDAttribute>
        <uro:BuildingIDAttribute>
          <uro:buildingID>14100-bldg-118593</uro:buildingID>
          <uro:prefecture codeSpace="../../codelists/Common_localPublicAuthorities.xml">14</uro:prefecture>
          <uro:city codeSpace="../../codelists/Common_localPublicAuthorities.xml">14103</uro:city>
        </uro:BuildingIDAttribute>
      </uro:buildingIDAttribute>
      <uro:bldgDataQualityAttribute>
        <uro:DataQualityAttribute>
          <uro:geometrySrcDescLod0 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod0>
          <uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod1>
          <uro:geometrySrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod2>
          <uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">999</uro:geometrySrcDescLod3>
          <uro:geometrySrcDescLod4 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">999</uro:geometrySrcDescLod4>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">201</uro:thematicSrcDesc>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">400</uro:thematicSrcDesc>
          <uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">023</uro:thematicSrcDesc>
          <uro:appearanceSrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">1</uro:appearanceSrcDescLod2>
          <uro:appearanceSrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">99</uro:appearanceSrcDescLod3>
          <uro:appearanceSrcDescLod4 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">99</uro:appearanceSrcDescLod4>
          <uro:lodType codeSpace="../../codelists/Building_lodType.xml">2.0</uro:lodType>
          <uro:lod1HeightType codeSpace="../../codelists/DataQualityAttribute_lod1HeightType.xml">2</uro:lod1HeightType>
          <uro:publicSurveyDataQualityAttribute>
            <uro:PublicSurveyDataQualityAttribute>
              <uro:srcScaleLod0 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod0>
              <uro:srcScaleLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod1>
              <uro:srcScaleLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod2>
              <uro:publicSurveySrcDescLod0 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod0>
              <uro:publicSurveySrcDescLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod1>
              <uro:publicSurveySrcDescLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod2>
            </uro:PublicSurveyDataQualityAttribute>
          </uro:publicSurveyDataQualityAttribute>
        </uro:DataQualityAttribute>
      </uro:bldgDataQualityAttribute>
    </bldg:Building>
  </core:cityObjectMember>
</core:CityModel>
