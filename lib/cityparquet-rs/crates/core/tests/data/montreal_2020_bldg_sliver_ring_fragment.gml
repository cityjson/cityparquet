<?xml version="1.0" encoding="UTF-8" standalone="no" ?>
<!--
  Test fragment from the Ville de Montréal 2020 3D building model (CityGML
  2.0 LoD2 with textures, RhinoCity export), Outremont tile O02_2020.gml in
  o01_2020_gml_01_03.zip from
  https://donnees.montreal.ca/dataset/batiments-3d-2020-maquette-lod2-avec-textures
  Licence: CC BY 4.0 (Ville de Montréal).

  Everything before the first cityObjectMember and ONE member, verbatim:
  bldg:Building 1351447 with its own app:appearance. One of its rings,
  UUID_578673a6-3812-4bad-87e1-38f605fd82da, is a sliver whose last
  position differs from its first below a millimetre, while its
  textureCoordinates open and close on the same UV pair: GML pairs the two
  lists position by position, so the ring and its UVs must lose their
  closing entry together, not each by its own equality test. The
  document's texture images are not included.
-->
<CityModel xmlns="http://www.opengis.net/citygml/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:core="http://www.opengis.net/citygml/base/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:tex="http://www.opengis.net/citygml/textures/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation=" http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/appearance/2.0  http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/bridge/2.0 http://schemas.opengis.net/citygml/bridge/2.0/bridge.xsd http://www.opengis.net/citygml/relief/2.0 http://schemas.opengis.net/citygml/relief/2.0/relief.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/texturedsurface/2.0 http://schemas.opengis.net/citygml/texturedsurface/2.0/texturedSurface.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/tunnel/2.0 http://schemas.opengis.net/citygml/tunnel/2.0/tunnel.xsd http://www.opengis.net/citygml/vegetation/2.0 http://schemas.opengis.net/citygml/vegetation/2.0/vegetation.xsd http://www.opengis.net/citygml/waterbody/2.0 http://schemas.opengis.net/citygml/waterbody/2.0/waterBody.xsd">

  <!-- File Written  With RhinoCity Software  CopyRight Rhinoterraain 2012 -->

  <gml:description>Exported by Rhinocity</gml:description>

  <gml:name>Essai Rhino</gml:name>

  <gml:boundedBy>
    <gml:Envelope srsDimension="3">
      <gml:lowerCorner>295579.468750 5041259.000000 66.753998
</gml:lowerCorner>
      <gml:upperCorner>297587.406250 5042639.000000 148.136002
</gml:upperCorner>
    </gml:Envelope>
  </gml:boundedBy>

  <cityObjectMember>
    <bldg:Building gml:id="1351447">
      <gen:stringAttribute name="parcelle">
        <gen:value> </gen:value>
      </gen:stringAttribute>
      <gen:doubleAttribute name="Volume">
        <gen:value>1385.098</gen:value>
      </gen:doubleAttribute>
      <gml:boundedBy>
        <gml:Envelope srsDimension="3">
          <gml:lowerCorner>297018.406250 5041844.500000 79.683006
</gml:lowerCorner>
          <gml:upperCorner>297033.343750 5041865.000000 94.439003
</gml:upperCorner>
        </gml:Envelope>
      </gml:boundedBy>
      <app:appearance>
        <app:Appearance>
          <app:theme>RhinoCity ObliqueTexturing</app:theme>
          <app:surfaceDataMember>
            <app:ParameterizedTexture>
              <app:imageURI>O02_2020_Appearance/1351447.jpg</app:imageURI>
              <app:textureType>specific</app:textureType>
              <app:wrapMode>border</app:wrapMode>
              <app:borderColor>0 0 0 1</app:borderColor>
              <app:target uri="#UUID_1085133a-89f2-41d9-9b92-4aafe632145c">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_644bfd96-aa68-4763-98ab-bdd90d9811c2">0.821540 0.541322 0.961937 0.684270 0.751389 0.765614 0.610989 0.622698 0.821540 0.541322 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_85fdc9d9-51f1-4515-b161-9172c2bb45cb">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_e65306e9-0268-4af1-b2cd-36c876a02df1">0.983666 0.778237 0.995250 0.790063 0.982977 0.794882 0.971429 0.783058 0.983666 0.778237 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_5e3deeb6-a695-4646-abd4-f260f6d1e7ff">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_62fb6e1f-30f2-4be9-87ea-d510a7a38c6a">0.060069 0.986327 0.062483 0.990267 0.059883 0.993720 0.060069 0.986327 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_89f79945-d5c1-405c-bdd9-643acc07274a">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_0bbd1fca-6956-4f73-9c81-6aabd589cbf2">0.060069 0.986327 0.059883 0.993720 0.046154 0.989944 0.054864 0.984848 0.060069 0.986327 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_334d9365-8fc9-4b03-8197-0a8056c8e54d">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_ba298c3d-9fe2-4d3a-9be9-68030c1f7345">0.054864 0.984848 0.046154 0.989944 0.048733 0.986211 0.054864 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_daf9d44d-9bfc-4b14-8201-7565d96a376b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_ed0f8af7-9442-4330-9681-a498cd50b225">0.048149 0.993316 0.059883 0.993720 0.054231 0.995092 0.048149 0.993316 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_38e61f99-681e-464f-95b6-64e026268adc">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_da09b49c-4d5a-4e04-a3ce-9d7762b3180c">0.046154 0.989944 0.059883 0.993720 0.048149 0.993316 0.046154 0.989944 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_c2307faf-4151-4002-a2f1-7e8f9a42f5c4">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_f337c0cd-f696-4493-8d28-6e9307e4d5bf">0.485696 0.778237 0.589868 0.884446 0.474805 0.782475 0.485696 0.778237 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_6fb9f200-e500-4eae-a5b2-c8b583f2dd50">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_8e947c43-2805-47db-8cd0-800902558bfc">0.474805 0.782475 0.589868 0.884446 0.497579 0.920171 0.393407 0.813965 0.474805 0.782475 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_47d20c3c-88bd-4946-8825-2cdce2de499b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_9dd283cf-e0cd-411a-adb4-7c2255ea015f">0.448047 0.413348 0.450568 0.415898 0.142786 0.538242 0.448047 0.413348 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_48fc7aee-db23-43b5-bd40-24e0f62f3a27">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_6a3e402c-b3df-4b67-920d-0c64aca7bce9">0.154409 0.521118 0.142786 0.538242 0.141335 0.526333 0.154409 0.521118 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_7aa91fe0-9e44-4513-8656-fab0604c911f">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_06cffd4b-b973-4796-8300-0924d72aba8f">0.128850 0.513716 0.141335 0.526333 0.142786 0.538242 0.128850 0.513716 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_86387691-bc82-4ac0-ae6d-77957f333eca">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_3ef2b689-8037-4a23-810e-43f36216adc1">0.448047 0.413348 0.154409 0.521118 0.141886 0.508500 0.448047 0.413348 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_aa6c17a6-f9a5-46bc-9ce4-c81796a3479a">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_870c30cf-c087-4fd0-bb85-eed0bce4a9d9">0.448047 0.413348 0.142786 0.538242 0.154409 0.521118 0.448047 0.413348 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_95c7e95e-0273-4f26-87ae-e48e101080af">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_1a661735-7414-4885-b325-ffa7fca6c38a">0.002198 0.395158 0.141886 0.508500 0.128850 0.513716 0.002198 0.395158 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_6b65c207-c760-4181-bb60-ec04224e7b8a">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_31d52548-b4a0-4cd3-a8ff-a13bf1007231">0.002198 0.395158 0.128850 0.513716 0.142786 0.538242 0.002198 0.395158 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_a1832430-80a7-4957-b8bf-5b1d5b950840">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_db928f2b-7c50-487f-bd64-6df4b28c5cf0">0.342682 0.306118 0.448047 0.413348 0.141886 0.508500 0.342682 0.306118 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_4f58ea87-9ce2-4903-afae-0a1a1f3f124b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_d5b773a4-60d0-4d68-9f5c-7d5bf239f746">0.342682 0.306118 0.141886 0.508500 0.002198 0.395158 0.342682 0.306118 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_d0788a63-544e-4618-9408-93df6ec4c35d">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_3e8fad30-d161-4f3d-9d02-aedc48383b16">0.230444 0.310511 0.227634 0.314714 0.002198 0.395158 0.230444 0.310511 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_4c88fcf8-8f51-445c-95d7-242d0d8b7c8d">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_c1ac101a-c380-44fe-8227-f3595664b1c9">0.309885 0.272727 0.230444 0.310511 0.002198 0.395158 0.309885 0.272727 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_ee2071d6-7970-4902-a840-c419c27c70f9">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_fe40a809-9e3e-412f-b67a-25dc5abbe165">0.227634 0.314714 0.229881 0.318490 0.002198 0.395158 0.227634 0.314714 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_6107c478-fb41-4938-8fcc-25767fe4ac67">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_6db26bd3-7be9-4fdc-bb04-7de2687ad6b8">0.229881 0.318490 0.236630 0.320454 0.002198 0.395158 0.229881 0.318490 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_59c10f9d-9db3-48dd-b34e-fc868cead758">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_fc53ffd8-6b8d-4a02-8e02-fd9fc98679a3">0.342682 0.306118 0.002198 0.395158 0.236630 0.320454 0.342682 0.306118 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_2c360db5-10a7-4553-86aa-6a2e6990afb2">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_2c4d2057-5081-44f8-8c28-f66b32c9d864">0.309885 0.272727 0.242989 0.310587 0.237212 0.308952 0.309885 0.272727 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_9df98b11-4635-44ea-b83b-c3ea9e4ed544">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_902bed18-bb48-4611-81fb-25ee9d4e5585">0.309885 0.272727 0.237212 0.308952 0.230444 0.310511 0.309885 0.272727 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_c1d49292-a0d1-431e-a1d4-5626eea163f4">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_ec124c3d-058a-4b69-994e-5a3fc51d3d96">0.309885 0.272727 0.245705 0.314998 0.242989 0.310587 0.309885 0.272727 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_3706a10d-135c-43d5-8fc1-c436bfe80d3b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_e6053040-e355-4998-9693-4a4b2e545f7a">0.342682 0.306118 0.242868 0.318887 0.245705 0.314998 0.342682 0.306118 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_8c0b9434-f85a-43a9-99db-1c15c839ca19">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_79e5bd03-476f-43f1-a438-975f4adfca65">0.309885 0.272727 0.342682 0.306118 0.245705 0.314998 0.309885 0.272727 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_a66972fb-15c3-4eec-a3cd-97155eb0c9e8">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_aa414c44-7129-4019-a909-7cfaf2c00e1a">0.342682 0.306118 0.236630 0.320454 0.242868 0.318887 0.342682 0.306118 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_2d5f2cf5-9ddd-4336-96ad-555c9b15e304">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_578673a6-3812-4bad-87e1-38f605fd82da">0.219780 0.984848 0.219780 0.984848 0.219780 0.984848 0.219780 0.984848 0.219780 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_a6c94df4-7a5a-433e-bed4-185459ade2b0">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_f7498c7b-274c-4f40-b15e-95786ed42f96">0.297399 0.694272 0.088957 0.774611 0.002198 0.744945 0.213294 0.663581 0.297399 0.694272 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_6106e27e-3cc6-4caa-8dee-b9e7be07135b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_d585e76e-96e4-4409-8d74-fd488b423a29">0.451706 0.634786 0.213294 0.663581 0.213294 0.663581 0.451706 0.634786 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_d1cde433-1df1-4b9e-93df-9795ab3302a3">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_0764596b-8640-4a47-9ce4-3d4b49b03be0">0.451706 0.634786 0.297399 0.694272 0.213294 0.663581 0.451706 0.634786 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_456b4595-a944-4ce2-8d33-2094b5f627af">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_20ba8efe-003e-47b7-bd2f-3d39f3677808">0.521456 0.541322 0.606021 0.575296 0.451706 0.634786 0.521456 0.541322 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_bb6fd529-d904-4cba-a1e2-28c745f988de">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_1266d690-f7b0-46bd-b7ba-eba4e5ac86d5">0.521456 0.541322 0.451706 0.634786 0.213294 0.663581 0.521456 0.541322 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_896ce53c-ceb8-47ab-89ee-3537b4f4f613">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_6e9e5b9c-e810-4667-ac39-7f14cf89680d">0.140659 0.984848 0.140659 0.984848 0.140659 0.984848 0.140659 0.984848 0.140659 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_0932c510-1bc6-4767-8507-d63387e65935">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_be6f4e10-1230-4e4f-863a-db7105dd8112">0.208879 0.185922 0.002198 0.269434 0.088411 0.205920 0.292511 0.123450 0.208879 0.185922 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_2ff56e0f-d805-45e1-b00c-3ee219073824">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_5d1238a8-bb20-4cb8-9b6b-f230fac3017d">0.510518 0.065644 0.208879 0.185922 0.292511 0.123450 0.594700 0.001377 0.510518 0.065644 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_a58624a2-f3a2-4c74-a83a-df839678793f">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_034b2b0c-21bf-4949-b225-78425b21238f">0.510518 0.065644 0.208879 0.185922 0.208879 0.185922 0.510518 0.065644 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_6496669d-5db2-414b-9a0c-ea3734a66e1a">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_799a1a3d-b2a6-4f83-a37d-b8448018999e">0.912890 0.778237 0.925110 0.790131 0.900183 0.810014 0.887912 0.798073 0.912890 0.778237 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_40cf0e0d-0d81-4629-92e8-cef0ba88a6fd">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_2582cda2-d78c-43e9-b4a6-2314bfa62194">0.040584 0.994024 0.028138 0.998829 0.002198 0.989607 0.014881 0.984848 0.040584 0.994024 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_bd86c675-7f85-4831-9fc7-d75dabf8193a">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_bbbc22ee-dc06-46c4-8d21-7b032626f076">0.151648 0.984848 0.151648 0.984848 0.151648 0.984848 0.151648 0.984848 0.151648 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_7f139108-8478-4a3c-8534-9f5a903562ca">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_d1025c58-19c5-45cb-9ddd-97750ca95095">0.942014 0.797812 0.929670 0.802853 0.954878 0.783116 0.966991 0.778237 0.942014 0.797812 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_b6581617-02ec-4ae2-a162-3ed1fa89f822">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_724896c3-dc39-495d-bcea-06d7b05cd5f7">0.087653 0.984848 0.092643 0.986181 0.086351 0.990846 0.081319 0.989539 0.087653 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_59ac28bd-0d48-4279-ae6f-d4ee43d343bf">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_9199ebc2-19fa-48e4-a398-2fb4649a4b5d">0.074004 0.984848 0.076296 0.988640 0.070413 0.993554 0.068132 0.989746 0.074004 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_db862037-e2e2-4a49-bbf5-5decb9eca891">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_eb682253-68a5-4313-aec7-56427d42f242">0.124865 0.987400 0.122584 0.990658 0.116484 0.988086 0.118824 0.984848 0.124865 0.987400 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_3021f928-1201-429c-b9ed-62798e2f88e9">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_b6f907f5-355e-48a6-8306-028a28bbaff4">0.174624 0.987163 0.169251 0.988345 0.162637 0.986006 0.168087 0.984848 0.174624 0.987163 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_a9c09069-5986-487c-9813-3ee57ba678fb">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_03ca0a6a-8b68-4a54-a6ae-2556f425f7e5">0.180220 0.984848 0.186084 0.986347 0.192698 0.988685 0.186885 0.987204 0.180220 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_780ef66f-a5e2-4789-9812-cef83419839f">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_7b9fffe5-8e58-4f43-832c-a200b8960621">0.129670 0.984848 0.129670 0.984848 0.129670 0.984848 0.129670 0.984848 0.129670 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_abb05d94-e741-4285-b9ed-15f937ae40b1">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_9d7c23ba-51e6-4c42-b8f1-8c80e6c7f830">0.197802 0.984848 0.197802 0.984848 0.197802 0.984848 0.197802 0.984848 0.197802 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_30a0e0b4-a03a-45b7-8679-a5f5ac211f1f">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_a2d3366b-2a8b-4b2b-8e51-bea40faa35d6">0.104724 0.989539 0.098901 0.990882 0.105316 0.986135 0.111058 0.984848 0.104724 0.989539 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_44491445-4755-425d-b721-c5ac92eeceef">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_ac82a829-8e54-436e-a3a0-dd78a0aae989">0.685566 0.816910 0.674951 0.821218 0.724800 0.782510 0.735330 0.778237 0.685566 0.816910 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_b43a1a85-651c-4838-b3a4-4ae80f326397">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_6bcc0558-8bff-4e93-8ab6-a253b701fbc8">0.674951 0.821218 0.595604 0.853232 0.646094 0.814265 0.724800 0.782510 0.674951 0.821218 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_43d93d58-402f-4a1e-816f-9a26e490cea4">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_61c58bbf-1b21-4fe3-a4f9-7ff528972a9e">0.282806 0.778237 0.388771 0.881156 0.339797 0.920995 0.232967 0.817239 0.282806 0.778237 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_ffee51ea-ef45-47f5-b787-50926153e35b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_afa870a9-40b7-43dc-8a96-c7354fed18c8">0.882829 0.798897 0.791676 0.833820 0.740659 0.813447 0.832560 0.778237 0.882829 0.798897 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_10b32328-b407-4aa2-adea-e28a36f6f235">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_866c212a-908e-495f-b308-33393e4fec5a">0.225432 0.913585 0.227977 0.916039 0.146197 0.982039 0.175515 0.953833 0.225432 0.913585 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_d940fb5d-775b-4231-9bc3-e3aa977119b8">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_c37b217a-33f2-4bd7-8528-190254dc91a5">0.143618 0.979552 0.175515 0.953833 0.146197 0.982039 0.143618 0.979552 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_a59ad729-d3a3-4923-8d38-04f1631fadee">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_e4433d7c-a1c2-4a0c-b9ca-027229a19dd9">0.002198 0.842399 0.068221 0.849785 0.035769 0.874967 0.002198 0.842399 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_fa952490-e603-4c2d-be5e-495fd8c2ad0b">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_9edd9aee-1628-49bd-a6de-bacaf4503042">0.085879 0.778237 0.119006 0.810376 0.068221 0.849785 0.085879 0.778237 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_26ab9ced-fc42-44ce-bcb3-504e2296fff7">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_3b35f3d4-6e5c-4787-a0bc-8fa636fbe50f">0.002198 0.842399 0.085879 0.778237 0.068221 0.849785 0.002198 0.842399 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_92f996d0-a9d3-4a33-8bc8-dccb34aed5bd">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_57850055-f30b-4e71-8e44-c0b73f516529">0.068221 0.849785 0.175515 0.953833 0.143618 0.979552 0.035769 0.874967 0.068221 0.849785 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
              <app:target uri="#UUID_570f0af0-69e2-47de-a7a3-6297ee1be9c6">
                <app:TexCoordList>
                  <app:textureCoordinates ring="#UUID_e7bb6240-0525-41ee-967e-67824c251552">0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 0.208791 0.984848 </app:textureCoordinates>
                </app:TexCoordList>
              </app:target>
            </app:ParameterizedTexture>
          </app:surfaceDataMember>
        </app:Appearance>
      </app:appearance>
      <bldg:measuredHeight uom="#m">14.756</bldg:measuredHeight>
      <bldg:lod2Solid>
        <gml:Solid gml:id="UUID_ec83df7e-1ca4-48de-a6fe-c70a64d4cafa">
          <gml:exterior>
            <gml:CompositeSurface gml:id="UUID_b831b93f-7817-49d9-aa51-2b7ff8b5bde9">
              <gml:surfaceMember xlink:href="#UUID_1085133a-89f2-41d9-9b92-4aafe632145c"/>
              <gml:surfaceMember xlink:href="#UUID_85fdc9d9-51f1-4515-b161-9172c2bb45cb"/>
              <gml:surfaceMember xlink:href="#UUID_5e3deeb6-a695-4646-abd4-f260f6d1e7ff"/>
              <gml:surfaceMember xlink:href="#UUID_89f79945-d5c1-405c-bdd9-643acc07274a"/>
              <gml:surfaceMember xlink:href="#UUID_334d9365-8fc9-4b03-8197-0a8056c8e54d"/>
              <gml:surfaceMember xlink:href="#UUID_daf9d44d-9bfc-4b14-8201-7565d96a376b"/>
              <gml:surfaceMember xlink:href="#UUID_38e61f99-681e-464f-95b6-64e026268adc"/>
              <gml:surfaceMember xlink:href="#UUID_c2307faf-4151-4002-a2f1-7e8f9a42f5c4"/>
              <gml:surfaceMember xlink:href="#UUID_6fb9f200-e500-4eae-a5b2-c8b583f2dd50"/>
              <gml:surfaceMember xlink:href="#UUID_47d20c3c-88bd-4946-8825-2cdce2de499b"/>
              <gml:surfaceMember xlink:href="#UUID_48fc7aee-db23-43b5-bd40-24e0f62f3a27"/>
              <gml:surfaceMember xlink:href="#UUID_7aa91fe0-9e44-4513-8656-fab0604c911f"/>
              <gml:surfaceMember xlink:href="#UUID_86387691-bc82-4ac0-ae6d-77957f333eca"/>
              <gml:surfaceMember xlink:href="#UUID_aa6c17a6-f9a5-46bc-9ce4-c81796a3479a"/>
              <gml:surfaceMember xlink:href="#UUID_95c7e95e-0273-4f26-87ae-e48e101080af"/>
              <gml:surfaceMember xlink:href="#UUID_6b65c207-c760-4181-bb60-ec04224e7b8a"/>
              <gml:surfaceMember xlink:href="#UUID_a1832430-80a7-4957-b8bf-5b1d5b950840"/>
              <gml:surfaceMember xlink:href="#UUID_4f58ea87-9ce2-4903-afae-0a1a1f3f124b"/>
              <gml:surfaceMember xlink:href="#UUID_d0788a63-544e-4618-9408-93df6ec4c35d"/>
              <gml:surfaceMember xlink:href="#UUID_4c88fcf8-8f51-445c-95d7-242d0d8b7c8d"/>
              <gml:surfaceMember xlink:href="#UUID_ee2071d6-7970-4902-a840-c419c27c70f9"/>
              <gml:surfaceMember xlink:href="#UUID_6107c478-fb41-4938-8fcc-25767fe4ac67"/>
              <gml:surfaceMember xlink:href="#UUID_59c10f9d-9db3-48dd-b34e-fc868cead758"/>
              <gml:surfaceMember xlink:href="#UUID_2c360db5-10a7-4553-86aa-6a2e6990afb2"/>
              <gml:surfaceMember xlink:href="#UUID_9df98b11-4635-44ea-b83b-c3ea9e4ed544"/>
              <gml:surfaceMember xlink:href="#UUID_c1d49292-a0d1-431e-a1d4-5626eea163f4"/>
              <gml:surfaceMember xlink:href="#UUID_3706a10d-135c-43d5-8fc1-c436bfe80d3b"/>
              <gml:surfaceMember xlink:href="#UUID_8c0b9434-f85a-43a9-99db-1c15c839ca19"/>
              <gml:surfaceMember xlink:href="#UUID_a66972fb-15c3-4eec-a3cd-97155eb0c9e8"/>
              <gml:surfaceMember xlink:href="#UUID_2d5f2cf5-9ddd-4336-96ad-555c9b15e304"/>
              <gml:surfaceMember xlink:href="#UUID_a6c94df4-7a5a-433e-bed4-185459ade2b0"/>
              <gml:surfaceMember xlink:href="#UUID_6106e27e-3cc6-4caa-8dee-b9e7be07135b"/>
              <gml:surfaceMember xlink:href="#UUID_d1cde433-1df1-4b9e-93df-9795ab3302a3"/>
              <gml:surfaceMember xlink:href="#UUID_456b4595-a944-4ce2-8d33-2094b5f627af"/>
              <gml:surfaceMember xlink:href="#UUID_bb6fd529-d904-4cba-a1e2-28c745f988de"/>
              <gml:surfaceMember xlink:href="#UUID_896ce53c-ceb8-47ab-89ee-3537b4f4f613"/>
              <gml:surfaceMember xlink:href="#UUID_0932c510-1bc6-4767-8507-d63387e65935"/>
              <gml:surfaceMember xlink:href="#UUID_2ff56e0f-d805-45e1-b00c-3ee219073824"/>
              <gml:surfaceMember xlink:href="#UUID_a58624a2-f3a2-4c74-a83a-df839678793f"/>
              <gml:surfaceMember xlink:href="#UUID_6496669d-5db2-414b-9a0c-ea3734a66e1a"/>
              <gml:surfaceMember xlink:href="#UUID_40cf0e0d-0d81-4629-92e8-cef0ba88a6fd"/>
              <gml:surfaceMember xlink:href="#UUID_bd86c675-7f85-4831-9fc7-d75dabf8193a"/>
              <gml:surfaceMember xlink:href="#UUID_7f139108-8478-4a3c-8534-9f5a903562ca"/>
              <gml:surfaceMember xlink:href="#UUID_b6581617-02ec-4ae2-a162-3ed1fa89f822"/>
              <gml:surfaceMember xlink:href="#UUID_59ac28bd-0d48-4279-ae6f-d4ee43d343bf"/>
              <gml:surfaceMember xlink:href="#UUID_db862037-e2e2-4a49-bbf5-5decb9eca891"/>
              <gml:surfaceMember xlink:href="#UUID_3021f928-1201-429c-b9ed-62798e2f88e9"/>
              <gml:surfaceMember xlink:href="#UUID_a9c09069-5986-487c-9813-3ee57ba678fb"/>
              <gml:surfaceMember xlink:href="#UUID_780ef66f-a5e2-4789-9812-cef83419839f"/>
              <gml:surfaceMember xlink:href="#UUID_abb05d94-e741-4285-b9ed-15f937ae40b1"/>
              <gml:surfaceMember xlink:href="#UUID_30a0e0b4-a03a-45b7-8679-a5f5ac211f1f"/>
              <gml:surfaceMember xlink:href="#UUID_44491445-4755-425d-b721-c5ac92eeceef"/>
              <gml:surfaceMember xlink:href="#UUID_b43a1a85-651c-4838-b3a4-4ae80f326397"/>
              <gml:surfaceMember xlink:href="#UUID_43d93d58-402f-4a1e-816f-9a26e490cea4"/>
              <gml:surfaceMember xlink:href="#UUID_ffee51ea-ef45-47f5-b787-50926153e35b"/>
              <gml:surfaceMember xlink:href="#UUID_10b32328-b407-4aa2-adea-e28a36f6f235"/>
              <gml:surfaceMember xlink:href="#UUID_d940fb5d-775b-4231-9bc3-e3aa977119b8"/>
              <gml:surfaceMember xlink:href="#UUID_a59ad729-d3a3-4923-8d38-04f1631fadee"/>
              <gml:surfaceMember xlink:href="#UUID_fa952490-e603-4c2d-be5e-495fd8c2ad0b"/>
              <gml:surfaceMember xlink:href="#UUID_26ab9ced-fc42-44ce-bcb3-504e2296fff7"/>
              <gml:surfaceMember xlink:href="#UUID_92f996d0-a9d3-4a33-8bc8-dccb34aed5bd"/>
              <gml:surfaceMember xlink:href="#UUID_570f0af0-69e2-47de-a7a3-6297ee1be9c6"/>
            </gml:CompositeSurface>
          </gml:exterior>
        </gml:Solid>
      </bldg:lod2Solid>
      <bldg:boundedBy>
        <bldg:RoofSurface gml:id="0615ddf9-9c1c-4f81-98a2-f1d2577fefff">
          <gen:doubleAttribute name="Area">
            <gen:value>127.455</gen:value>
          </gen:doubleAttribute>
          <bldg:lod2MultiSurface>
            <gml:MultiSurface gml:id="UUID_9553b69b-a923-4aa5-8d97-b9114cb10c40" srsDimension="3">
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_1085133a-89f2-41d9-9b92-4aafe632145c">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_644bfd96-aa68-4763-98ab-bdd90d9811c2">
                      <gml:posList>297027.83700000 5041850.28400000 90.86000000 297021.99600000 5041854.13600000 90.86000000 297018.43300000 5041848.73300000 90.86000000 297024.27300000 5041844.88100000 90.86000000 297027.83700000 5041850.28400000 90.86000000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_85fdc9d9-51f1-4515-b161-9172c2bb45cb">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_e65306e9-0268-4af1-b2cd-36c876a02df1">
                      <gml:posList>297023.22900000 5041854.08300000 94.43900000 297022.71400000 5041854.42600000 94.43900000 297022.48900000 5041854.08600000 94.43900000 297023.00400000 5041853.74400000 94.43900000 297023.22900000 5041854.08300000 94.43900000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_5e3deeb6-a695-4646-abd4-f260f6d1e7ff">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_62fb6e1f-30f2-4be9-87ea-d510a7a38c6a">
                      <gml:posList>297031.50000000 5041856.55600000 92.31600000 297031.31900000 5041856.63200000 92.31600000 297031.15600000 5041856.56100000 92.31600000 297031.50000000 5041856.55600000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_89f79945-d5c1-405c-bdd9-643acc07274a">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_0bbd1fca-6956-4f73-9c81-6aabd589cbf2">
                      <gml:posList>297031.50000000 5041856.55600000 92.31600000 297031.15600000 5041856.56100000 92.31600000 297031.31900000 5041856.15500000 92.31600000 297031.56400000 5041856.40200000 92.31600000 297031.50000000 5041856.55600000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_334d9365-8fc9-4b03-8197-0a8056c8e54d">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_ba298c3d-9fe2-4d3a-9be9-68030c1f7345">
                      <gml:posList>297031.56400000 5041856.40200000 92.31600000 297031.31900000 5041856.15500000 92.31600000 297031.49500000 5041856.22500000 92.31600000 297031.56400000 5041856.40200000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_daf9d44d-9bfc-4b14-8201-7565d96a376b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_ed0f8af7-9442-4330-9681-a498cd50b225">
                      <gml:posList>297031.16400000 5041856.21800000 92.31600000 297031.15600000 5041856.56100000 92.31600000 297031.08700000 5041856.39800000 92.31600000 297031.16400000 5041856.21800000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_38e61f99-681e-464f-95b6-64e026268adc">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_da09b49c-4d5a-4e04-a3ce-9d7762b3180c">
                      <gml:posList>297031.31900000 5041856.15500000 92.31600000 297031.15600000 5041856.56100000 92.31600000 297031.16400000 5041856.21800000 92.31600000 297031.31900000 5041856.15500000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_c2307faf-4151-4002-a2f1-7e8f9a42f5c4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_f337c0cd-f696-4493-8d28-6e9307e4d5bf">
                      <gml:posList>297033.32700000 5041861.57100000 86.90800000 297028.95300000 5041864.45600000 86.90800000 297033.14000000 5041861.28900000 86.90800000 297033.32700000 5041861.57100000 86.90800000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_6fb9f200-e500-4eae-a5b2-c8b583f2dd50">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_8e947c43-2805-47db-8cd0-800902558bfc">
                      <gml:posList>297033.14000000 5041861.28900000 86.90800000 297028.95300000 5041864.45600000 86.90800000 297027.37600000 5041862.06600000 86.90800000 297031.75000000 5041859.18100000 86.90800000 297033.14000000 5041861.28900000 86.90800000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_47d20c3c-88bd-4946-8825-2cdce2de499b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_9dd283cf-e0cd-411a-adb4-7c2255ea015f">
                      <gml:posList>297027.37600025 5041862.06600038 91.46399280 297027.27199950 5041862.13499924 91.46401452 297021.99599950 5041854.13599924 90.86001453 297027.37600025 5041862.06600038 91.46399280 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_48fc7aee-db23-43b5-bd40-24e0f62f3a27">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_6a3e402c-b3df-4b67-920d-0c64aca7bce9">
                      <gml:posList>297022.71400000 5041854.42600000 90.90019061 297021.99599950 5041854.13599924 90.86001453 297022.48900000 5041854.08600000 90.87449165 297022.71400000 5041854.42600000 90.90019061 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_7aa91fe0-9e44-4513-8656-fab0604c911f">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_06cffd4b-b973-4796-8300-0924d72aba8f">
                      <gml:posList>297023.00400000 5041853.74400000 90.87436695 297022.48900000 5041854.08600000 90.87449165 297021.99599950 5041854.13599924 90.86001453 297023.00400000 5041853.74400000 90.87436695 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_86387691-bc82-4ac0-ae6d-77957f333eca">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_3ef2b689-8037-4a23-810e-43f36216adc1">
                      <gml:posList>297027.37600025 5041862.06600038 91.46399280 297022.71400000 5041854.42600000 90.90019061 297023.22900000 5041854.08300000 90.90001329 297027.37600025 5041862.06600038 91.46399280 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_aa6c17a6-f9a5-46bc-9ce4-c81796a3479a">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_870c30cf-c087-4fd0-bb85-eed0bce4a9d9">
                      <gml:posList>297027.37600025 5041862.06600038 91.46399280 297021.99599950 5041854.13599924 90.86001453 297022.71400000 5041854.42600000 90.90019061 297027.37600025 5041862.06600038 91.46399280 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_95c7e95e-0273-4f26-87ae-e48e101080af">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_1a661735-7414-4885-b325-ffa7fca6c38a">
                      <gml:posList>297027.83699950 5041850.28399924 90.86001453 297023.22900000 5041854.08300000 90.90001329 297023.00400000 5041853.74400000 90.87436695 297027.83699950 5041850.28399924 90.86001453 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_6b65c207-c760-4181-bb60-ec04224e7b8a">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_31d52548-b4a0-4cd3-a8ff-a13bf1007231">
                      <gml:posList>297027.83699950 5041850.28399924 90.86001453 297023.00400000 5041853.74400000 90.87436695 297021.99599950 5041854.13599924 90.86001453 297027.83699950 5041850.28399924 90.86001453 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_a1832430-80a7-4957-b8bf-5b1d5b950840">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_db928f2b-7c50-487f-bd64-6df4b28c5cf0">
                      <gml:posList>297031.75000107 5041859.18100162 91.46396914 297027.37600025 5041862.06600038 91.46399280 297023.22900000 5041854.08300000 90.90001329 297031.75000107 5041859.18100162 91.46396914 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_4f58ea87-9ce2-4903-afae-0a1a1f3f124b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_d5b773a4-60d0-4d68-9f5c-7d5bf239f746">
                      <gml:posList>297031.75000107 5041859.18100162 91.46396914 297023.22900000 5041854.08300000 90.90001329 297027.83699950 5041850.28399924 90.86001453 297031.75000107 5041859.18100162 91.46396914 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_d0788a63-544e-4618-9408-93df6ec4c35d">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_3e8fad30-d161-4f3d-9d02-aedc48383b16">
                      <gml:posList>297031.49500000 5041856.22500000 91.29957353 297031.31900000 5041856.15500000 91.28978253 297027.83699950 5041850.28399924 90.86001453 297031.49500000 5041856.22500000 91.29957353 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_4c88fcf8-8f51-445c-95d7-242d0d8b7c8d">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_c1ac101a-c380-44fe-8227-f3595664b1c9">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297031.49500000 5041856.22500000 91.29957353 297027.83699950 5041850.28399924 90.86001453 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_ee2071d6-7970-4902-a840-c419c27c70f9">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_fe40a809-9e3e-412f-b67a-25dc5abbe165">
                      <gml:posList>297031.31900000 5041856.15500000 91.28978253 297031.16400000 5041856.21800000 91.28771881 297027.83699950 5041850.28399924 90.86001453 297031.31900000 5041856.15500000 91.28978253 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_6107c478-fb41-4938-8fcc-25767fe4ac67">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_6db26bd3-7be9-4fdc-bb04-7de2687ad6b8">
                      <gml:posList>297031.16400000 5041856.21800000 91.28771881 297031.08700000 5041856.39800000 91.29451846 297027.83699950 5041850.28399924 90.86001453 297031.16400000 5041856.21800000 91.28771881 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_59c10f9d-9db3-48dd-b34e-fc868cead758">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_fc53ffd8-6b8d-4a02-8e02-fd9fc98679a3">
                      <gml:posList>297031.75000107 5041859.18100162 91.46396914 297027.83699950 5041850.28399924 90.86001453 297031.08700000 5041856.39800000 91.29451846 297031.75000107 5041859.18100162 91.46396914 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_2c360db5-10a7-4553-86aa-6a2e6990afb2">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_2c4d2057-5081-44f8-8c28-f66b32c9d864">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297031.50000000 5041856.55600000 91.31716445 297031.56400000 5041856.40200000 91.31128181 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_9df98b11-4635-44ea-b83b-c3ea9e4ed544">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_902bed18-bb48-4611-81fb-25ee9d4e5585">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297031.56400000 5041856.40200000 91.31128181 297031.49500000 5041856.22500000 91.29957353 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_c1d49292-a0d1-431e-a1d4-5626eea163f4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_ec124c3d-058a-4b69-994e-5a3fc51d3d96">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297031.31900000 5041856.63200000 91.31488255 297031.50000000 5041856.55600000 91.31716445 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_3706a10d-135c-43d5-8fc1-c436bfe80d3b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_e6053040-e355-4998-9693-4a4b2e545f7a">
                      <gml:posList>297031.75000107 5041859.18100162 91.46396914 297031.15600000 5041856.56100000 91.30549006 297031.31900000 5041856.63200000 91.31488255 297031.75000107 5041859.18100162 91.46396914 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_8c0b9434-f85a-43a9-99db-1c15c839ca19">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_79e5bd03-476f-43f1-a438-975f4adfca65">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297031.75000107 5041859.18100162 91.46396914 297031.31900000 5041856.63200000 91.31488255 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_a66972fb-15c3-4eec-a3cd-97155eb0c9e8">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_aa414c44-7129-4019-a909-7cfaf2c00e1a">
                      <gml:posList>297031.75000107 5041859.18100162 91.46396914 297031.08700000 5041856.39800000 91.29451846 297031.15600000 5041856.56100000 91.30549006 297031.75000107 5041859.18100162 91.46396914 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_2d5f2cf5-9ddd-4336-96ad-555c9b15e304">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_578673a6-3812-4bad-87e1-38f605fd82da">
                      <gml:posList>297027.83699950 5041850.28399924 90.86001453 297021.99599950 5041854.13599924 90.86001453 297021.99600000 5041854.13600000 90.86000000 297027.83700000 5041850.28400000 90.86000000 297027.83700000 5041850.28400000 90.86000000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:RoofSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:WallSurface gml:id="b18732ba-c647-43b9-87fd-6bf2542020e7">
          <gen:doubleAttribute name="Area">
            <gen:value>575.422</gen:value>
          </gen:doubleAttribute>
          <bldg:lod2MultiSurface>
            <gml:MultiSurface gml:id="UUID_2bcd65ee-3bc8-424f-a5b3-42f23ee23e09" srsDimension="3">
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_a6c94df4-7a5a-433e-bed4-185459ade2b0">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_f7498c7b-274c-4f40-b15e-95786ed42f96">
                      <gml:posList>297021.99600000 5041854.13600000 79.68300762 297018.43300000 5041848.73300000 79.68300762 297018.43300000 5041848.73300000 90.86000000 297021.99600000 5041854.13600000 90.86000000 297021.99600000 5041854.13600000 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_6106e27e-3cc6-4caa-8dee-b9e7be07135b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_d585e76e-96e4-4409-8d74-fd488b423a29">
                      <gml:posList>297024.63399950 5041858.13549924 79.68300762 297021.99600000 5041854.13600000 90.86000000 297021.99599950 5041854.13599924 90.86001453 297024.63399950 5041858.13549924 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_d1cde433-1df1-4b9e-93df-9795ab3302a3">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_0764596b-8640-4a47-9ce4-3d4b49b03be0">
                      <gml:posList>297024.63399950 5041858.13549924 79.68300762 297021.99600000 5041854.13600000 79.68300762 297021.99600000 5041854.13600000 90.86000000 297024.63399950 5041858.13549924 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_456b4595-a944-4ce2-8d33-2094b5f627af">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_20ba8efe-003e-47b7-bd2f-3d39f3677808">
                      <gml:posList>297027.27199950 5041862.13499924 91.46401452 297027.27199950 5041862.13499924 79.68300762 297024.63399950 5041858.13549924 79.68300762 297027.27199950 5041862.13499924 91.46401452 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_bb6fd529-d904-4cba-a1e2-28c745f988de">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_1266d690-f7b0-46bd-b7ba-eba4e5ac86d5">
                      <gml:posList>297027.27199950 5041862.13499924 91.46401452 297024.63399950 5041858.13549924 79.68300762 297021.99599950 5041854.13599924 90.86001453 297027.27199950 5041862.13499924 91.46401452 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_896ce53c-ceb8-47ab-89ee-3537b4f4f613">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_6e9e5b9c-e810-4667-ac39-7f14cf89680d">
                      <gml:posList>297024.27300000 5041844.88100000 90.86000000 297018.43300000 5041848.73300000 90.86000000 297018.43300000 5041848.73300000 79.68300762 297024.27300000 5041844.88100000 79.68300762 297024.27300000 5041844.88100000 90.86000000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_0932c510-1bc6-4767-8507-d63387e65935">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_be6f4e10-1230-4e4f-863a-db7105dd8112">
                      <gml:posList>297027.83700000 5041850.28400000 90.86000000 297024.27300000 5041844.88100000 90.86000000 297024.27300000 5041844.88100000 79.68300762 297027.83700000 5041850.28400000 79.68300762 297027.83700000 5041850.28400000 90.86000000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_2ff56e0f-d805-45e1-b00c-3ee219073824">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_5d1238a8-bb20-4cb8-9b6b-f230fac3017d">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297027.83700000 5041850.28400000 90.86000000 297027.83700000 5041850.28400000 79.68300762 297033.11200070 5041858.28300105 79.68300762 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_a58624a2-f3a2-4c74-a83a-df839678793f">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_034b2b0c-21bf-4949-b225-78425b21238f">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297027.83699950 5041850.28399924 90.86001453 297027.83700000 5041850.28400000 90.86000000 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_6496669d-5db2-414b-9a0c-ea3734a66e1a">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_799a1a3d-b2a6-4f83-a37d-b8448018999e">
                      <gml:posList>297023.22900000 5041854.08300000 90.90001329 297022.71400000 5041854.42600000 90.90019061 297022.71400000 5041854.42600000 94.43900000 297023.22900000 5041854.08300000 94.43900000 297023.22900000 5041854.08300000 90.90001329 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_40cf0e0d-0d81-4629-92e8-cef0ba88a6fd">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_2582cda2-d78c-43e9-b4a6-2314bfa62194">
                      <gml:posList>297022.71400000 5041854.42600000 90.90019061 297022.48900000 5041854.08600000 90.87449165 297022.48900000 5041854.08600000 94.43900000 297022.71400000 5041854.42600000 94.43900000 297022.71400000 5041854.42600000 90.90019061 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_bd86c675-7f85-4831-9fc7-d75dabf8193a">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_bbbc22ee-dc06-46c4-8d21-7b032626f076">
                      <gml:posList>297023.00400000 5041853.74400000 94.43900000 297022.48900000 5041854.08600000 94.43900000 297022.48900000 5041854.08600000 90.87449165 297023.00400000 5041853.74400000 90.87436695 297023.00400000 5041853.74400000 94.43900000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_7f139108-8478-4a3c-8534-9f5a903562ca">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_d1025c58-19c5-45cb-9ddd-97750ca95095">
                      <gml:posList>297023.22900000 5041854.08300000 94.43900000 297023.00400000 5041853.74400000 94.43900000 297023.00400000 5041853.74400000 90.87436695 297023.22900000 5041854.08300000 90.90001329 297023.22900000 5041854.08300000 94.43900000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_b6581617-02ec-4ae2-a162-3ed1fa89f822">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_724896c3-dc39-495d-bcea-06d7b05cd5f7">
                      <gml:posList>297031.56400000 5041856.40200000 91.31128181 297031.50000000 5041856.55600000 91.31716445 297031.50000000 5041856.55600000 92.31600000 297031.56400000 5041856.40200000 92.31600000 297031.56400000 5041856.40200000 91.31128181 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_59ac28bd-0d48-4279-ae6f-d4ee43d343bf">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_9199ebc2-19fa-48e4-a398-2fb4649a4b5d">
                      <gml:posList>297031.50000000 5041856.55600000 91.31716445 297031.31900000 5041856.63200000 91.31488255 297031.31900000 5041856.63200000 92.31600000 297031.50000000 5041856.55600000 92.31600000 297031.50000000 5041856.55600000 91.31716445 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_db862037-e2e2-4a49-bbf5-5decb9eca891">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_eb682253-68a5-4313-aec7-56427d42f242">
                      <gml:posList>297031.31900000 5041856.63200000 91.31488255 297031.15600000 5041856.56100000 91.30549006 297031.15600000 5041856.56100000 92.31600000 297031.31900000 5041856.63200000 92.31600000 297031.31900000 5041856.63200000 91.31488255 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_3021f928-1201-429c-b9ed-62798e2f88e9">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_b6f907f5-355e-48a6-8306-028a28bbaff4">
                      <gml:posList>297031.15600000 5041856.56100000 91.30549006 297031.08700000 5041856.39800000 91.29451846 297031.08700000 5041856.39800000 92.31600000 297031.15600000 5041856.56100000 92.31600000 297031.15600000 5041856.56100000 91.30549006 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_a9c09069-5986-487c-9813-3ee57ba678fb">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_03ca0a6a-8b68-4a54-a6ae-2556f425f7e5">
                      <gml:posList>297031.16400000 5041856.21800000 92.31600000 297031.08700000 5041856.39800000 92.31600000 297031.08700000 5041856.39800000 91.29451846 297031.16400000 5041856.21800000 91.28771881 297031.16400000 5041856.21800000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_780ef66f-a5e2-4789-9812-cef83419839f">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_7b9fffe5-8e58-4f43-832c-a200b8960621">
                      <gml:posList>297031.31900000 5041856.15500000 92.31600000 297031.16400000 5041856.21800000 92.31600000 297031.16400000 5041856.21800000 91.28771881 297031.31900000 5041856.15500000 91.28978253 297031.31900000 5041856.15500000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_abb05d94-e741-4285-b9ed-15f937ae40b1">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_9d7c23ba-51e6-4c42-b8f1-8c80e6c7f830">
                      <gml:posList>297031.49500000 5041856.22500000 92.31600000 297031.31900000 5041856.15500000 92.31600000 297031.31900000 5041856.15500000 91.28978253 297031.49500000 5041856.22500000 91.29957353 297031.49500000 5041856.22500000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_30a0e0b4-a03a-45b7-8679-a5f5ac211f1f">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_a2d3366b-2a8b-4b2b-8e51-bea40faa35d6">
                      <gml:posList>297031.56400000 5041856.40200000 92.31600000 297031.49500000 5041856.22500000 92.31600000 297031.49500000 5041856.22500000 91.29957353 297031.56400000 5041856.40200000 91.31128181 297031.56400000 5041856.40200000 92.31600000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_44491445-4755-425d-b721-c5ac92eeceef">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_ac82a829-8e54-436e-a3a0-dd78a0aae989">
                      <gml:posList>297033.32700000 5041861.57100000 86.90800000 297033.14000000 5041861.28900000 86.90800000 297033.14000000 5041861.28900000 79.68300762 297033.32700000 5041861.57100000 79.68300762 297033.32700000 5041861.57100000 86.90800000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_b43a1a85-651c-4838-b3a4-4ae80f326397">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_6bcc0558-8bff-4e93-8ab6-a253b701fbc8">
                      <gml:posList>297033.14000000 5041861.28900000 86.90800000 297031.75000000 5041859.18100000 86.90800000 297031.75000000 5041859.18100000 79.68300762 297033.14000000 5041861.28900000 79.68300762 297033.14000000 5041861.28900000 86.90800000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_43d93d58-402f-4a1e-816f-9a26e490cea4">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_61c58bbf-1b21-4fe3-a4f9-7ff528972a9e">
                      <gml:posList>297033.32700000 5041861.57100000 79.68300762 297028.95300000 5041864.45600000 79.68300762 297028.95300000 5041864.45600000 86.90800000 297033.32700000 5041861.57100000 86.90800000 297033.32700000 5041861.57100000 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_ffee51ea-ef45-47f5-b787-50926153e35b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_afa870a9-40b7-43dc-8a96-c7354fed18c8">
                      <gml:posList>297028.95300000 5041864.45600000 79.68300762 297027.37600000 5041862.06600000 79.68300762 297027.37600000 5041862.06600000 86.90800000 297028.95300000 5041864.45600000 86.90800000 297028.95300000 5041864.45600000 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_10b32328-b407-4aa2-adea-e28a36f6f235">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_866c212a-908e-495f-b308-33393e4fec5a">
                      <gml:posList>297027.37600000 5041862.06600000 79.68300762 297027.27199950 5041862.13499924 79.68300762 297027.27199950 5041862.13499924 91.46401452 297027.37600000 5041862.06600000 86.90800000 297027.37600000 5041862.06600000 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_d940fb5d-775b-4231-9bc3-e3aa977119b8">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_c37b217a-33f2-4bd7-8528-190254dc91a5">
                      <gml:posList>297027.37600025 5041862.06600038 91.46399280 297027.37600000 5041862.06600000 86.90800000 297027.27199950 5041862.13499924 91.46401452 297027.37600025 5041862.06600038 91.46399280 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_a59ad729-d3a3-4923-8d38-04f1631fadee">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_e4433d7c-a1c2-4a0c-b9ca-027229a19dd9">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297031.75000000 5041859.18100000 86.90800000 297031.75000107 5041859.18100162 91.46396914 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_fa952490-e603-4c2d-be5e-495fd8c2ad0b">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_9edd9aee-1628-49bd-a6de-bacaf4503042">
                      <gml:posList>297033.11200070 5041858.28300105 79.68300762 297031.75000000 5041859.18100000 79.68300762 297031.75000000 5041859.18100000 86.90800000 297033.11200070 5041858.28300105 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_26ab9ced-fc42-44ce-bcb3-504e2296fff7">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_3b35f3d4-6e5c-4787-a0bc-8fa636fbe50f">
                      <gml:posList>297033.11200070 5041858.28300105 91.46397996 297033.11200070 5041858.28300105 79.68300762 297031.75000000 5041859.18100000 86.90800000 297033.11200070 5041858.28300105 91.46397996 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_92f996d0-a9d3-4a33-8bc8-dccb34aed5bd">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_57850055-f30b-4e71-8e44-c0b73f516529">
                      <gml:posList>297031.75000000 5041859.18100000 86.90800000 297027.37600000 5041862.06600000 86.90800000 297027.37600025 5041862.06600038 91.46399280 297031.75000107 5041859.18100162 91.46396914 297031.75000000 5041859.18100000 86.90800000 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:WallSurface>
      </bldg:boundedBy>
      <bldg:boundedBy>
        <bldg:GroundSurface gml:id="5ee46e2c-92a7-4944-b145-94a023110a55">
          <gen:doubleAttribute name="Area">
            <gen:value>127.323</gen:value>
          </gen:doubleAttribute>
          <bldg:lod2MultiSurface>
            <gml:MultiSurface gml:id="UUID_fccc9e77-d0be-43d5-883a-52246d5a3b54" srsDimension="3">
              <gml:surfaceMember>
                <gml:Polygon gml:id="UUID_570f0af0-69e2-47de-a7a3-6297ee1be9c6">
                  <gml:exterior>
                    <gml:LinearRing gml:id="UUID_e7bb6240-0525-41ee-967e-67824c251552">
                      <gml:posList>297033.14000000 5041861.28900000 79.68300762 297031.75000000 5041859.18100000 79.68300762 297033.11200070 5041858.28300105 79.68300762 297027.83700000 5041850.28400000 79.68300762 297024.27300000 5041844.88100000 79.68300762 297018.43300000 5041848.73300000 79.68300762 297021.99600000 5041854.13600000 79.68300762 297024.63399950 5041858.13549924 79.68300762 297027.27199950 5041862.13499924 79.68300762 297027.37600000 5041862.06600000 79.68300762 297028.95300000 5041864.45600000 79.68300762 297033.32700000 5041861.57100000 79.68300762 297033.14000000 5041861.28900000 79.68300762 </gml:posList>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </bldg:lod2MultiSurface>
        </bldg:GroundSurface>
      </bldg:boundedBy>
    </bldg:Building>
  </cityObjectMember>
</CityModel>
