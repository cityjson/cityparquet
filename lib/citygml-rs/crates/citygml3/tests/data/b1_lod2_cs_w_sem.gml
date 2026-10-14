<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<CityModel xmlns:con="http://www.opengis.net/citygml/construction/3.0" xmlns:tran="http://www.opengis.net/citygml/transportation/3.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/3.0" xmlns:veg="http://www.opengis.net/citygml/vegetation/3.0" xmlns="http://www.opengis.net/citygml/3.0" xmlns:dem="http://www.opengis.net/citygml/relief/3.0" xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:bldg="http://www.opengis.net/citygml/building/3.0" xmlns:ct="urn:oasis:names:tc:ciq:ct:3" xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/3.0" xmlns:dyn="http://www.opengis.net/citygml/dynamizer/3.0" xmlns:pcl="http://www.opengis.net/citygml/pointcloud/3.0" xmlns:tun="http://www.opengis.net/citygml/tunnel/3.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/3.0" xmlns:gen="http://www.opengis.net/citygml/generics/3.0" xmlns:xAL="urn:oasis:names:tc:ciq:xal:3" xmlns:app="http://www.opengis.net/citygml/appearance/3.0" xmlns:luse="http://www.opengis.net/citygml/landuse/3.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:brid="http://www.opengis.net/citygml/bridge/3.0" xmlns:vers="http://www.opengis.net/citygml/versioning/3.0" xmlns:xlink="http://www.w3.org/1999/xlink" xsi:schemaLocation="http://www.opengis.net/citygml/construction/3.0 http://schemas.opengis.net/citygml/construction/3.0/construction.xsd http://www.opengis.net/citygml/transportation/3.0 http://schemas.opengis.net/citygml/transportation/3.0/transportation.xsd http://www.opengis.net/citygml/waterbody/3.0 http://schemas.opengis.net/citygml/waterbody/3.0/waterBody.xsd http://www.opengis.net/citygml/vegetation/3.0 http://schemas.opengis.net/citygml/vegetation/3.0/vegetation.xsd http://www.opengis.net/citygml/relief/3.0 http://schemas.opengis.net/citygml/relief/3.0/relief.xsd http://www.opengis.net/citygml/building/3.0 http://schemas.opengis.net/citygml/building/3.0/building.xsd http://www.opengis.net/citygml/cityobjectgroup/3.0 http://schemas.opengis.net/citygml/cityobjectgroup/3.0/cityObjectGroup.xsd http://www.opengis.net/citygml/dynamizer/3.0 http://schemas.opengis.net/citygml/dynamizer/3.0/dynamizer.xsd http://www.opengis.net/citygml/pointcloud/3.0 http://schemas.opengis.net/citygml/pointcloud/3.0/pointCloud.xsd http://www.opengis.net/citygml/tunnel/3.0 http://schemas.opengis.net/citygml/tunnel/3.0/tunnel.xsd http://www.opengis.net/citygml/cityfurniture/3.0 http://schemas.opengis.net/citygml/cityfurniture/3.0/cityFurniture.xsd http://www.opengis.net/citygml/generics/3.0 http://schemas.opengis.net/citygml/generics/3.0/generics.xsd http://www.opengis.net/citygml/appearance/3.0 http://schemas.opengis.net/citygml/appearance/3.0/appearance.xsd http://www.opengis.net/citygml/landuse/3.0 http://schemas.opengis.net/citygml/landuse/3.0/landUse.xsd http://www.opengis.net/citygml/bridge/3.0 http://schemas.opengis.net/citygml/bridge/3.0/bridge.xsd http://www.opengis.net/citygml/versioning/3.0 http://schemas.opengis.net/citygml/versioning/3.0/versioning.xsd">
  <cityObjectMember>
    <bldg:Building gml:id="ID_19c73636-0ffd-39a1-a106-67ec6093b288">
      <boundary>
        <con:GroundSurface gml:id="b_ground_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_ground">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>0.0 0.0 0.0</gml:pos>
                      <gml:pos>0.0 100.0 0.0</gml:pos>
                      <gml:pos>100.0 100.0 0.0</gml:pos>
                      <gml:pos>100.0 0.0 0.0</gml:pos>
                      <gml:pos>0.0 0.0 0.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:GroundSurface>
      </boundary>
      <boundary>
        <con:WallSurface gml:id="b_wall_1_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_wall_1">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>0.0 0.0 0.0</gml:pos>
                      <gml:pos>100.0 0.0 0.0</gml:pos>
                      <gml:pos>100.0 0.0 100.0</gml:pos>
                      <gml:pos>0.0 0.0 100.0</gml:pos>
                      <gml:pos>0.0 0.0 0.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:WallSurface>
      </boundary>
      <boundary>
        <con:WallSurface gml:id="b_wall_2_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_wall_2">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>100.0 0.0 0.0</gml:pos>
                      <gml:pos>100.0 100.0 0.0</gml:pos>
                      <gml:pos>100.0 100.0 100.0</gml:pos>
                      <gml:pos>100.0 0.0 100.0</gml:pos>
                      <gml:pos>100.0 0.0 0.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:WallSurface>
      </boundary>
      <boundary>
        <con:WallSurface gml:id="b_wall_3_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_wall_3">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>100.0 100.0 0.0</gml:pos>
                      <gml:pos>0.0 100.0 0.0</gml:pos>
                      <gml:pos>0.0 100.0 100.0</gml:pos>
                      <gml:pos>100.0 100.0 100.0</gml:pos>
                      <gml:pos>100.0 100.0 0.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:WallSurface>
      </boundary>
      <boundary>
        <con:WallSurface gml:id="b_wall_4_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_wall_4">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>0.0 100.0 0.0</gml:pos>
                      <gml:pos>0.0 0.0 0.0</gml:pos>
                      <gml:pos>0.0 0.0 100.0</gml:pos>
                      <gml:pos>0.0 100.0 100.0</gml:pos>
                      <gml:pos>0.0 100.0 0.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:WallSurface>
      </boundary>
      <boundary>
        <con:RoofSurface gml:id="b_roof_1_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_roof_1">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>0.0 0.0 100.0</gml:pos>
                      <gml:pos>100.0 0.0 100.0</gml:pos>
                      <gml:pos>50.0 0.0 150.0</gml:pos>
                      <gml:pos>0.0 0.0 100.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:RoofSurface>
      </boundary>
      <boundary>
        <con:RoofSurface gml:id="b_roof_2_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_roof_2">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>100.0 100.0 100.0</gml:pos>
                      <gml:pos>0.0 100.0 100.0</gml:pos>
                      <gml:pos>50.0 100.0 150.0</gml:pos>
                      <gml:pos>100.0 100.0 100.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:RoofSurface>
      </boundary>
      <boundary>
        <con:RoofSurface gml:id="b_roof_3_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_roof_3">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>100.0 0.0 100.0</gml:pos>
                      <gml:pos>100.0 100.0 100.0</gml:pos>
                      <gml:pos>50.0 100.0 150.0</gml:pos>
                      <gml:pos>50.0 0.0 150.0</gml:pos>
                      <gml:pos>100.0 0.0 100.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:RoofSurface>
      </boundary>
      <boundary>
        <con:RoofSurface gml:id="b_roof_4_sem">
          <lod2MultiSurface>
            <gml:MultiSurface>
              <gml:surfaceMember>
                <gml:Polygon gml:id="b_roof_4">
                  <gml:exterior>
                    <gml:LinearRing>
                      <gml:pos>0.0 0.0 100.0</gml:pos>
                      <gml:pos>50.0 0.0 150.0</gml:pos>
                      <gml:pos>50.0 100.0 150.0</gml:pos>
                      <gml:pos>0.0 100.0 100.0</gml:pos>
                      <gml:pos>0.0 0.0 100.0</gml:pos>
                    </gml:LinearRing>
                  </gml:exterior>
                </gml:Polygon>
              </gml:surfaceMember>
            </gml:MultiSurface>
          </lod2MultiSurface>
        </con:RoofSurface>
      </boundary>
      <lod2Solid>
        <gml:CompositeSolid>
          <gml:solidMember>
            <gml:Solid>
              <gml:exterior>
                <gml:Shell>
                  <gml:surfaceMember xlink:href="#b_ground"/>
                  <gml:surfaceMember xlink:href="#b_wall_1"/>
                  <gml:surfaceMember xlink:href="#b_wall_2"/>
                  <gml:surfaceMember xlink:href="#b_wall_3"/>
                  <gml:surfaceMember xlink:href="#b_wall_4"/>
                  <gml:surfaceMember xlink:href="#internal-ceiling"/>
                </gml:Shell>
              </gml:exterior>
            </gml:Solid>
          </gml:solidMember>
          <gml:solidMember>
            <gml:Solid>
              <gml:exterior>
                <gml:Shell>
                  <gml:surfaceMember>
                    <gml:OrientableSurface orientation="-">
                      <gml:baseSurface xlink:href="#internal-ceiling"/>
                    </gml:OrientableSurface>
                  </gml:surfaceMember>
                  <gml:surfaceMember xlink:href="#b_roof_1"/>
                  <gml:surfaceMember xlink:href="#b_roof_2"/>
                  <gml:surfaceMember xlink:href="#b_roof_3"/>
                  <gml:surfaceMember xlink:href="#b_roof_4"/>
                </gml:Shell>
              </gml:exterior>
            </gml:Solid>
          </gml:solidMember>
        </gml:CompositeSolid>
      </lod2Solid>
      <lod2MultiSurface>
        <gml:MultiSurface>
          <gml:surfaceMember>
            <gml:Polygon gml:id="internal-ceiling">
              <gml:exterior>
                <gml:LinearRing>
                  <gml:pos>0.0 0.0 100.0</gml:pos>
                  <gml:pos>100.0 0.0 100.0</gml:pos>
                  <gml:pos>100.0 100.0 100.0</gml:pos>
                  <gml:pos>0.0 100.0 100.0</gml:pos>
                  <gml:pos>0.0 0.0 100.0</gml:pos>
                </gml:LinearRing>
              </gml:exterior>
            </gml:Polygon>
          </gml:surfaceMember>
        </gml:MultiSurface>
      </lod2MultiSurface>
    </bldg:Building>
  </cityObjectMember>
</CityModel>